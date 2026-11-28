# How Zalỏ works

This is the map of the app as it is built now: what runs where, how a tap in the UI reaches Zalo, how messages come
back, and how the pieces are tested. The README has the short version; the protocol details are in the other files in
`docs/`.

## 1. The pieces

| Piece | Language | Role |
|---|---|---|
| `android/` | Kotlin, Jetpack Compose | The app: screens, notifications, the call screen, microphone, camera, Opus codec |
| `rust/zalo-proto` | Rust | Zalo's web protocol: QR and cookie login, request encryption, the chat WebSocket, the HTTP API (a port of zca-js) |
| `rust/zalo-media` | Rust | Call media (zrtc signalling, SRTP, RTCP, H.264 packets) and history-sync formats (ZDB archives, LZMA, a read-only SQLite reader) |
| `rust/zalo-server` | Rust | `ZaloSession` (the logged-in account), `ChatStore` (the message cache), the call and history-sync managers, and the REST/WebSocket API on top |
| `rust/zalo-android` | Rust | `libzalocore.so`: the JNI functions the app calls |
| `rust/zalo-cli` | Rust | `zalom3ui-server`: the same core as a standalone HTTP server, with the web UI in `public/` |

There is one core, `zalo-server`, with two front doors: JNI inside the app, and TCP for the standalone server. Both go
through the same router, so every request is validated and answered by the same code either way.

## 2. One process, no socket

The app runs in **one Android process**. The Rust core is loaded into it as a library and called directly.

```
 UI (Compose, AppViewModel) ─┐
 MessageNotifier ────────────┼──► ApiClient ──► Transport ──┬─ NativeTransport ──JNI──► ZaloCore (libzalocore.so)
 CallService / CallActivity ─┘                              │
                                                            └─ HttpTransport (OkHttp) ──► zalom3ui-server elsewhere
```

- **`ZaloCore`** (`android/.../server/ZaloCore.kt`) is a process-wide singleton. `ensureStarted(context)` loads the
  library, opens the message store and starts the session; it is idempotent, and whoever needs the core first starts
  it (the screen, the notifier, a call, or the service after a reboot). `shutdown()` flushes the store and stops it;
  `restart()` does both, in the same process.
- **`ApiClient`** has one method per API call (send a message, react, create a group, …) and parses the JSON answers
  into the app's models. It does not know how the request travels: that is the `Transport`.
  - `NativeTransport` (Zalo runs on this phone) calls the JNI functions below.
  - `HttpTransport` (*Settings → Zalo runs on → Remote server*) sends the same requests to a server over HTTP, with its
    address and token.
  - `ApiClient.onDevice(context)` and `ApiClient.forSettings(context, settings)` pick the right one.
- **`ZaloService`** is a foreground service ("Connected to Zalo"), used in *Stay connected → Always* (section 6). It
  keeps the process alive while the app is closed, runs `MessageNotifier` (new-message notifications), and calls
  `POST /api/retry` whenever Android reports that a network came back. `BootReceiver` starts it after a reboot. It no
  longer owns the core: stopping it does not stop Zalo for an open screen.
- **`StayConnectedCoordinator`** connects and disconnects the core in the other two modes, and
  **`ZaloNotificationListener`** is the notification bridge (section 6).
- **`CallService`** is a second foreground service (microphone, and camera for video) that runs while a call is active.
- **`NotificationActionReceiver`** and **`CallActionReceiver`** handle *Reply*, *Mark as read* and *Decline* from
  notifications. **`OtpCopyReceiver`** handles *Copy 482910* (section 12).

Before the Rust port, the core was Node.js in a separate `:node` process, reached over a loopback port with a random
token. Node could only start once per process, which forced that split. With Rust it is gone: there is no port, no
token, and nothing on the phone listens on the network.

## 3. How a request travels

Example: you send "hi" in a 1:1 chat.

1. `ApiClient.send("user", id, "hi")` builds `POST /api/threads/user/<id>/messages` with a JSON body.
2. `NativeTransport` calls `ZaloCore.request(method, pathAndQuery, contentType, body)` on a background thread. The JNI
   call blocks until the answer is ready.
3. In Rust (`rust/zalo-android/src/lib.rs`), the call runs on the core's tokio runtime:
   `EmbeddedCore::request` (`rust/zalo-server/src/embedded.rs`) hands the request to the axum router with
   `tower::oneshot`. No TCP is involved.
4. The router's handler (`http.rs`) checks the body and calls `ZaloSession::send_message`.
5. `ZaloSession` calls the Zalo client (`zalo-proto`), which encrypts the parameters the way Zalo Web does and sends them
   to Zalo's HTTPS endpoint. The message is saved in `ChatStore` and broadcast to listeners as an event.
6. The answer comes back as one byte array: `u16 status | u16 content-type length | content type | body`. Status `0`
   means the call itself failed (core not running, bad argument), and Kotlin throws an `IOException`. Otherwise
   `ApiClient` maps the status and JSON exactly as it does for HTTP.

Large uploads (photos, videos, voice) are not passed as one big byte array: Kotlin copies the picked file to a temporary
file in the app's cache and calls `requestFile(...)`, which streams it into the request body in 64 KiB chunks.

The router used in-process leaves out the network checks the TCP server needs (Host and Origin checks, the access
token, browser security headers). Only the app itself can call JNI. Body-size limits stay.

**Panics.** Every JNI entry point is wrapped in `catch_unwind`, and release builds unwind on panic
(`rust/Cargo.toml`). A bug that panics in Rust fails that request (or that background task) instead of closing the app.

## 4. Events: how news reaches the screen

The core pushes events: a new message, a reaction, a recall, "typing…", a read receipt, a call ringing, history-sync
progress, a status change.

- Over TCP they go out on the `/ws` WebSocket as JSON text.
- In the app, the same stream comes from `ZaloCore.eventsOpen()` / `eventsNext(handle, timeoutMs)` /
  `eventsClose(handle)`. `eventsNext` waits up to the timeout and returns one JSON event, `null` on timeout, or `""`
  when the subscription has ended (closed, or the core stopped). Both come from one function in `http.rs`
  (`EventStream`): the first event is always `{"type":"status", …}`, then every broadcast.
- `ApiClient.events()` turns this into a Kotlin `Flow`. The screen and `MessageNotifier` each hold one subscription.

This is a **pull model**: Kotlin threads call into Rust and wait. Rust never calls back into the JVM, which avoids
attaching Rust threads to the JVM and holding references to Kotlin objects.

A message event carries `notify: true` (and the chat name) when the phone should show a notification: not your own
message, not a muted chat, not the chat you have open (`PUT /api/presence`, refreshed while a chat is on screen), and
not replayed history. An incoming message also carries `otp` (a code from its card, or found in its plain text; the
detector is `extract_otp_code` in `store.rs`, mirrored by `extractOtpCode` in `ui/OtpCode.kt` with the same test vectors).

## 5. The connection to Zalo

`ZaloSession` (`rust/zalo-server/src/session.rs`) is the logged-in account. Its state is one of `logged_out`,
`awaiting_qr_scan`, `logging_in`, `ready`, `disconnected` and `paused` (logged in, but deliberately not connected:
`POST /api/listener/pause`; `POST /api/listener/resume` connects again).

- **Login.** `POST /api/login` starts a QR login (`zalo-proto/src/login.rs`): the QR image is in `GET /api/status`
  until you scan it and confirm in the official app. The session cookies and device id are then saved to
  `credentials.json` (written atomically, mode `0600`).
- **Resume.** On start the core logs back in with the saved cookies. If the phone is offline it keeps the session and
  retries (5 s growing to 60 s); `POST /api/retry` retries at once. Only a session Zalo rejects asks for a new QR code.
- **The chat WebSocket.** Once logged in, the core opens Zalo's chat socket (`zalo-proto/src/listener.rs`). On every
  `Connected` it asks Zalo to replay the messages and reactions it missed (`request_old_messages` /
  `request_old_reactions`, 1:1 and groups). Replayed messages are stored as history and do not notify (except inside
  a notification-bridge window, section 6).
- **Replay report.** Each connect writes a report (`rust/zalo-server/src/replay.rs`): how long the listener was
  offline (kept on disk, so it survives the process being killed), the newest message stored per kind before the
  replay, every batch Zalo sent back (count, oldest and newest time and id, how many were already stored), and a
  verdict per kind: `covered` (the replay reaches back to what we had, so no gap), `possible_gap` (a full batch that
  doesn't reach back), `nothing_missed` or `unknown`. `GET /api/debug/replay-log` shows the last 50. No message text.
- **Replay mode** (`PUT /api/debug/replay-mode`), for the replay test: `null` asks once with `lastId: null` (what Zalo
  Web is assumed to do, and the default); `newest` sends the newest stored message id as `lastId`; `page` asks again
  with the oldest id of a full batch that doesn't overlap, up to 5 times.
- **Drops.** If the socket closes unexpectedly, the session reconnects with one tracked task: 5 s, doubling up to
  5 minutes, with a little jitter, reset after a successful connect. `POST /api/retry` (sent by the app when the network
  comes back) skips the wait. Logout, a new login and `POST /api/reconnect` cancel it.
- **Session ended.** Before every reconnect attempt (and after a 3003 kick) the core re-logs in with the saved cookies.
  If Zalo refuses the session (a login error code, a Zalo API error, HTTP 401/403) it stops reconnecting, drops the
  client and goes `logged_out` with `loggedOutReason: "elsewhere"` (the status `error` is *You were logged out on
  another device. Log in again.*). A saved session rejected at start-up is `loggedOutReason: "expired"`. The chats and
  `credentials.json` stay on disk; the app shows the login screen with a note. Network errors, 5xx, 408 and 429 never
  count: the core keeps reconnecting. `loggedOutReason` is optional (absent in older cores) and only set while
  `logged_out`.
- **Kicked.** Zalo allows one web connection per account. If Zalo Web or PC connects elsewhere, Zalo closes Zalỏ's socket
  as a duplicate. The session does not fight back: it shows *Zalo Web was opened somewhere else* and waits for you to
  tap *Reconnect*.
- **Control poll.** Some accounts get call signalling and history-sync messages on a separate long-poll channel
  (`zalo-proto/src/api/ctrlpoll.rs`) instead of the chat socket. The call manager runs that poll when the login offers
  it, and feeds both the call manager and history sync. If the chat socket gets kicked while the poll runs, the poll
  is turned off until the next logout, in case it caused the kick.

## 6. Stay connected

*Settings → Stay connected* (`android/.../data/StayConnected.kt`, `server/StayConnectedCoordinator.kt`) decides when the
core holds the chat socket. It only applies when Zalo runs on the phone.

- **Always** (the default, and what existing installs get): `ZaloService` runs in the foreground and `BootReceiver`
  starts it after a reboot. Section 2.
- **While open:** no foreground service and no boot start. The coordinator follows the app's foreground state
  (`ProcessLifecycleOwner`): when the app comes to the front it starts the core and calls `listener/resume`, and the
  replay brings in what was missed. A minute after the app leaves the front it calls `listener/pause`. It never pauses
  while a call is ringing or active, while `CallService` runs, or while an attachment or voice upload is in flight; it
  checks again every few seconds. While connected, `MessageNotifier` runs, so messages and calls still notify.
- **While open + notification bridge:** as *While open*, plus `ZaloNotificationListener`, a
  `NotificationListenerService` that sees the official Zalo app's (`com.zing.zalo`) notifications. The component is
  disabled in the manifest and only enabled in this mode, and Android binds it only after you grant notification
  access. When Zalo posts a message notification, the coordinator:
  1. takes a short partial wake lock;
  2. starts the core;
  3. opens a **notify window** (`PUT /api/notify-window`): for its duration, replayed messages newer than the Zalo
     notification (minus 2 minutes) notify, as long as they are unread, not muted, not yours and not on screen. This is
     because here the replay *is* the news;
  4. starts the notifier, then resumes the listener;
  5. pauses again after a minute without another Zalo notification (a burst extends the window).

  With *Hide Zalo's notifications* on, Zalo's notification for that chat is cancelled once Zalỏ has posted its own.
  Notifications that are calls (category call or missed call, alarm, or a full-screen intent) are never touched. The
  startup log records only the package, category and classification of Zalo's notifications, never their text. All of
  this runs inside the listener service Android keeps bound; it never starts a foreground service from the background.

`ZaloNotificationListener` has a second, independent job, **Codes from Zalo's notifications** (a separate setting, off by
default, section 12). The component is enabled when the bridge mode wants it *or* that setting is on
(`ListenerJobs.componentEnabled`), and each job is gated on its own setting (`ListenerJobs.jobsFor`): the bridge only in
its mode, the code rescue in any mode, and the rescue never brings the core online.

The decisions (when to pause or resume, coalescing, which Zalo notifications count, which listener job runs) are in plain
Kotlin classes (`ListenerGate`, `BusyCheck`, `ZaloNotificationRules`, `ListenerJobs`) with their own unit tests. The default for new installs is one
constant (`StayConnected.NEW_INSTALL_DEFAULT`), to change once the replay test ([replay-test.md](replay-test.md))
shows that *While open* doesn't lose 1:1 history.

What only a real phone can show: whether the bridge gets network access in Doze when Zalo's notification arrives (the
biggest open risk; nothing exempts Zalỏ from Doze), whether Zalo's notifications are classified and matched to chats
correctly, and whether a bridge connect disconnects Zalo Web/PC.

## 7. The message store

Everything is in the app's private storage under `files/zalo/` (on a server: `DATA_DIR`).

| File | What |
|---|---|
| `credentials.json` | The saved session. Equivalent to being logged in. |
| `messages/meta.json` | The chat list: names, avatars, pinned, muted, unread marks, read positions |
| `messages/threads/<hex>.json` | One file per chat, named by the hex of `kind:id` (e.g. `user:100`); the last 500 messages, up to 2,000 with history synced from the phone |
| `call-trace.jsonl` | A redacted log of call signalling, for *Settings → Developer → Share call log* |
| `replay-log.json`, `replay-mode.json`, `listener-state.json` | The last 50 replay reports, the replay mode, and when the listener was last connected |
| `socket-log.jsonl` | The last 500 WebSocket frames, structure only (no message text) |
| `server.log`, `startup.log` | The core's log (rotated) and the app's startup steps, for *Share log* |

`ChatStore` saves only the chats that changed, each file written atomically. A file that can't be read is set aside
and the rest still loads. Search (`GET /api/search`) folds case and Vietnamese diacritics ("duc" finds "Đức").

## 8. Calls

1:1 voice and video calls. The core does the signalling and the network; the app does the audio and video hardware.

```
 mic ─► AudioRecord ─► Opus encoder ─► CallAudioLink ──link──► core ─► SRTP ─► zrtc over UDP ─► Zalo's media server
 speaker ◄─ AudioTrack ◄─ Opus decoder ◄─ jitter buffer ◄─ CallAudioLink ◄──link── core ◄─ …
 camera ─► H.264 encoder ─► CallAudioLink ──► core ─► RTP packets ─► …
```

- **Signalling** (`rust/zalo-server/src/routes/calls.rs`, `zalo-proto/src/api/voicecall.rs`): incoming calls arrive as
  `voip` controls on the chat socket or the control poll; outgoing calls go through Zalo's call API. The call manager
  tracks ringing, answered and ended calls, and sends `call` events.
- **Media** (`rust/zalo-media/src/call/`): zrtc (Zalo's media protocol) over UDP, SRTP encryption, RTCP, H.264
  packetizing and reassembly.
- **The link to the app** carries binary frames: Opus audio (`0x01` in, `0x02` out), H.264 access units (`0x03` in,
  `0x04` out) and key-frame requests (`0x05`, `0x06`). In the app it is `ZaloCore.audioOpen()` / `audioSend` /
  `audioNext` / `audioClose`; with a remote server it is the `/ws/call-audio` WebSocket. Same frames either way
  ([call-audio.md](call-audio.md)).
- **In the app**: `CallService`, `CallAudioEngine`, `OpusFrameCodec` (via `libzaloopus.so`), `JitterBuffer`,
  `CameraVideoSender`, `VideoDecoder`, and the call screen in `ui/call/`.

Group calls are not supported. Protocol notes: [calls.md](calls.md), [video-calls.md](video-calls.md).

## 9. History sync from the phone

Zalo PC can ask the official Zalo app to send its chat history. Zalỏ does the same
(`rust/zalo-server/src/routes/history_sync.rs`): it sends a public key, the phone asks you to allow it, uploads an
encrypted backup, and the core downloads, decrypts (`zalo-media/src/sync/zdb.rs`), unpacks (`lzma.rs`), reads the
SQLite database directly (`sqliteread.rs`, no libsqlite3) and imports the messages. *Settings → Sync messages from
phone*. It follows Zalo PC's protocol and has been confirmed working on a real phone ([history-sync.md](history-sync.md)).

## 10. Remote-server mode and the standalone server

`zalom3ui-server` (`rust/zalo-cli`) runs the same core over TCP. Because it is on the network, it adds what the
in-process path leaves out: Host and Origin checks, an access token (required unless it listens on loopback),
security headers and a CSP for the web UI. The app talks to it with `HttpTransport` and the `/ws` and `/ws/call-audio`
WebSockets. The web UI in `public/` is optional (`PUBLIC_DIR`).

## 11. One-time codes and the socket log

**Frames.** `dispatch_frame` (`zalo-proto/src/listener.rs`) reads `pageMsgs` (Official Account, ZNS and OTP messages) on
501 and 510 whatever the subCmd, as the TypeScript version did, and on the offline re-push commands (504, 513, 514, 524,
534, 551, 570). `msgs` / `groupMsgs` are only read where they were before, so nothing is emitted twice. A 501 `pageMsgs`
is live (`Messages`); on 510/1 it is part of the paged replay (`OldMessages`), and on 510 with another subCmd it is
history outside the paged replay (`OldPageMessages`: stored, never notifies). The normalizer follows the TS `toRawMessage`
(a null id falls through to the next id field, a numeric `msgType` is kept as a string, an unusable `ts` becomes now).

**What a real test showed, and asking after an empty frame.** When an Official Account OTP reached the phone, the
web session got one `cmd=501` frame with every list empty (`msgs`, `pageMsgs`, ...): Zalo signals that something changed
but leaves the message out. The code still reaches Zalỏ through *Codes from Zalo's notifications* (confirmed on a real
phone, with Copy, and Zalo's duplicate notification hidden). To get it without the official app, the session answers
such an empty 501 by asking for missed messages, the same replay request it sends on connect (`request_old_messages`,
1:1, `lastId` null). It does so at most once per 30 s, never within 30 s of connecting, and opens a one-minute notify
window so a recent message in the answer still notifies. The answer (the next 1:1 replay batch within 15 s) is kept out
of the connect's replay report. The socket log marks the frame with `zalỏ: empty frame: asked Zalo for missed
messages`; the next `cmd=510` entry shows whether the answer held the message. Whether Zalo includes OA messages in
that answer is what a real test will tell.

**Socket log.** Each decoded frame produces a structure-only `FrameReport` (`zalo-proto/src/framelog.rs`) that the
listener sends as `WsEvent::Frame` before the frame's other events; `ZaloSession` keeps the last 500 in
`socket_log.rs` and appends them to `socket-log.jsonl` (compacted, so it survives a restart). One entry has: time,
version, cmd, subCmd, whether a handler consumed the frame (`handled`) or it fell through to `RawFrame`, the top-level
and `data` key names, every array with its length and whether any handler reads it, for control frames the
`act_type/act`, and per message item the `msgType`, the keys of `content` (or `string`, `string>{keys}` for JSON in a
string, `number`, ...), whether `uidFrom` is 0 / own / other, and the outcome: message, undo or dropped with the reason
(`no msgId`, `thread id empty/0`, `undo content`, `not an object`, `own message (selfListen off)`). Non-array fields
of the payload and of `data` are listed as `values:` (numbers and booleans in full, e.g. `more=1`,
`lastActionId=…`; strings only as `str(N)`, their length; objects as their key names). A frame that can't be decoded
also shows its outer wrapper (`envelope: encrypt=2, data=str(N)`) and a short reason (`why:`). The server's echo of
the keepalive ping (cmd 2) is shown as a handled frame, not an error. Message text, content values, names and phone
numbers are never recorded; every string from the wire is reduced to `[A-Za-z0-9._:-]`.
`GET /api/debug/socket-log` renders it (newest first), `DELETE` clears it; *Settings → Developer → Socket log* views,
copies, shares and clears it.

## 12. Codes in notifications

- **Detection** (`extract_otp_code` / `extractOtpCode`): on folded lowercase text, per line: a strong keyword (OTP, mã xác
  thực, verification code, ...) followed in the same sentence by the first code-like number; or a weak keyword directly
  followed by the code (`code: 123456`, `mã: 123456`, `mã của bạn là 1234`); or a code-like number right before a keyword
  (`739201 là mã xác nhận`, `123456 is your verification code`, `G-123456 is your Google verification code`); or a line
  that is only a code under a keyword line. Code-like means 4-8 digits, or two groups of three (`482 910`, `482-910`,
  returned as `482910`). Amounts, dates, times, 9+ digit numbers, numbers after tk/stk/sdt/hotline, and a year after
  "năm" are skipped.
- **Chat notifications.** `MessageNotifier` shows a message that carries a code with the code first, a **Copy 482910**
  action and the lock-screen privacy (`VISIBILITY_PRIVATE` with a public version that says only "New code received").
- **Copy.** The action is a broadcast to `OtpCopyReceiver`, which writes the clipboard with
  `ClipDescription.EXTRA_IS_SENSITIVE` (Android 13+; the same key is set on older versions, where it is ignored). It
  never reads the clipboard and never starts an activity, so Android 10+'s background-clipboard rule (reading only) and
  Android 12+'s notification-trampoline rule do not apply, on any API level. Android 13+ shows its own "copied"
  confirmation; Zalỏ adds a toast on 12 and older.
- **Codes from Zalo's notifications** (`OtpLogic.kt`, `OtpNotifications.kt`, `ZaloNotificationListener.kt`): for a Zalo
  message notification, the listener reads the title, text, big text, inbox lines and the recent MessagingStyle messages in
  memory, finds a code the same way, and posts "Code 482910 · title" with Copy on its own channel. `OtpDedupe` skips a code
  Zalỏ announced itself in the last 2 minutes, and a code already rescued in the last 10 minutes (Zalo re-posting a
  notification with more lines). The text and the code are never logged or stored; the startup log says only "otp
  rescued from zalo notification".

## 13. Building and testing

- **Build.** `cd android && ./gradlew assembleRelease`. The Gradle task `buildRustCore<Variant>` runs
  `cargo build --release --target aarch64-linux-android -p zalo-android` with the NDK's clang and packs
  `libzalocore.so` into the APK. The Opus codec (`libzaloopus.so`) is built from C with CMake. The app is arm64-only.
  The release APK is about 5.7 MB.
- **Rust tests** (`cd rust && cargo test --workspace`, 222 tests):
  - protocol and media units (encryption, frames, SRTP, RTCP, H.264, LZMA, SQLite and ZDB against real fixtures in
    `rust/fixtures/`);
  - the store (persistence, migration, corrupt files, search);
  - the session against a **fake Zalo client** (`rust/zalo-server/src/testing.rs`, `testing` feature): it records every
    call the session makes and lets a test script Zalo's answers, push socket events and drive the login, so login
    races, receipts, recalls, the reconnect backoff, the control poll, the replay report and modes, pause/resume and
    the notify window are tested without a Zalo account;
  - the HTTP router, the WebSocket and call-audio sockets, and the embedded core (in-process requests, events, the
    audio link, starting again after shutdown).
- **Android tests** (`./gradlew testDebugUnitTest`, 209 JVM tests): models, formatting, the transports over a fake native
  layer, the call-audio link, jitter buffer, voice recorder, layout and gesture logic, and the *Stay connected*
  decisions (pause/resume timing, busy checks, which Zalo notifications count), the code detector (same vectors as
  Rust), code de-duplication and which notification-listener job runs.
- **CI** (`.github/workflows/ci.yml`) runs both, builds the standalone server and smoke-tests it, builds debug and release
  APKs, checks the native libraries are inside, and uploads the APKs.

Confirmed on a real phone so far: the JNI path end to end (the app running the core in-process) and history sync. What
only a real phone and account can still check: call audio and video quality, the notification bridge in Doze, and how much history Zalo replays after time offline ([replay-test.md](replay-test.md)).
