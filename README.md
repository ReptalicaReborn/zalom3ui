# Zalỏ

**Zalỏ** is a third-party Android app (Material 3) for a **personal Zalo account**. It started from the Zalo Lite app of
[watchhrept](https://github.com/ReptalicaReborn/watchhrept) and grew the media, notification, call, friend and group
features it was missing.

It logs in the same way Zalo Web (chat.zalo.me) does. Everything that talks to Zalo is a Rust library,
`libzalocore.so`, that runs **inside the app**. There is no server to set up and no computer needed. The same Rust
code also builds as a small standalone server (`zalom3ui-server`) with a web UI, for running Zalo somewhere other than
the phone.

> [!WARNING]
> This is **unofficial**. It is not affiliated with or endorsed by Zalo/VNG, and using it is against Zalo's
> Terms of Service. Zalo may lock or ban accounts that use it. Use a secondary account if you can, and expect it
> to break when Zalo changes its web protocol.

## How it works

```
 ┌──────────────────────────── the Zalỏ app (one Android process) ───────────────────────────┐
 │                                                                                           │
 │  Compose UI ─┐                                                                            │
 │  notifier  ──┼─► ApiClient ─► NativeTransport ──JNI──► libzalocore.so (Rust)              │
 │  calls     ──┘                                          ├─ REST router (in-process)       │
 │                                                         ├─ ZaloSession + ChatStore ───────┼──► Zalo's web endpoints
 │  ZaloService: keeps the process alive in the background └─ calls (SRTP over UDP) ─────────┼──► Zalo's media servers
 └───────────────────────────────────────────────────────────────────────────────────────────┘
```

- **One process, no socket.** The UI calls the Rust core directly over JNI. Requests go through the same REST router the
  standalone server uses, but in memory: no port, no token, nothing listening on the phone.
- **Login:** scan a QR code with the official Zalo app, as for chat.zalo.me. The session is kept in the app's private
  storage, so restarts log back in on their own, also after the phone was offline.
- **Real-time:** the core holds Zalo's chat WebSocket. By default a foreground service ("Connected to Zalo") keeps it
  open while the app is closed and starts it again after a reboot, so notifications and incoming calls work and 1:1
  messages keep landing in the history (*Settings → Stay connected* can change that, below). If the connection drops,
  it retries with a growing delay (5 s up to 5 min), and right away when the network comes back.
- **History:** Zalo Web has no history endpoint for 1:1 chats, so the core keeps its own cache: the last 500 messages
  per chat (2,000 for history synced from the phone). On every reconnect it asks Zalo to replay what it missed. Group
  chats also pull recent history from Zalo when opened. *Sync messages from phone* can ask the official Zalo app to
  hand over older history ([docs/history-sync.md](docs/history-sync.md); confirmed working on a real phone).
- **Calls:** 1:1 voice and video calls run in the core (Zalo's zrtc media protocol, SRTP, H.264). The app does the
  microphone, speaker, camera and Opus codec ([docs/calls.md](docs/calls.md), [docs/call-audio.md](docs/call-audio.md),
  [docs/video-calls.md](docs/video-calls.md)).

The details (how a request travels, events, calls, storage, the connection lifecycle and the tests) are in
[docs/architecture.md](docs/architecture.md).

> [!IMPORTANT]
> Zalo allows only **one web connection per account**. Opening chat.zalo.me or Zalo PC elsewhere disconnects Zalỏ
> (the app shows a banner with a *Reconnect* button), and reconnecting Zalỏ disconnects them.

## Features

| Feature | How in the app |
|---|---|
| Chat list | Pinned section, muted icon, unread and `@` badges, check marks on your last message, live "typing…" |
| Search | Search icon on the chat list: chats, contacts, groups and message text (diacritics optional: "duc" finds "Đức") |
| New chat | Pencil button → search any friend or group |
| Read & reply | Message field with an emoji panel; drafts are kept per chat |
| Reply to a message (quote) | Swipe the message to the right, or long-press → **Reply** |
| Reactions | Long-press → ❤️ 👍 😆 😮 😭 😡 above the menu (tap yours again to remove) |
| Copy, forward, select several | Long-press → **Copy text** / **Forward** → pick a chat |
| Recall for everyone / delete for me | Long-press → confirm |
| @mention in groups | Type `@` and pick a member |
| Seen / delivered | Check marks in your bubbles: one circle sent, two delivered, two filled seen |
| Pin, mute, mark read/unread, delete conversation | Long-press the chat in the list, or ⋮ in the chat |
| Photos, videos, GIFs, stickers, files, links, contact cards, locations | Shown in the chat; tap a photo to see it full screen |
| Send photos & videos, files, camera shots | **+** next to the message field (HEIC photos are sent as JPEG); or share into Zalỏ from another app |
| Voice messages | Hold the mic button to record; voice messages play in the chat |
| Stickers | **+** → *Sticker*: Zalo's sticker search, with suggested words |
| Polls (groups) | Create, vote, add options, close |
| Official accounts, ZNS, OTP messages | Shown as cards with their buttons. One-time codes (in cards and in plain text) get a one-tap copy button in the chat and a **Copy 482910** action in the notification, which keeps the code off the lock screen |
| Calls | 1:1 voice and video calls, incoming calls with a pop-up or full screen on the lock screen (*Settings → Incoming calls*) |
| Notifications | One per chat with its latest lines; **Reply** and **Mark as read** from the notification. Muted chats and the chat you have open stay quiet |
| Codes from Zalo's notifications | *Settings → Codes from Zalo's notifications* (off by default): if the official Zalo app is on this phone, Zalỏ reads one-time codes from its notifications and shows them with a Copy button, in case Zalo never sends them to Zalỏ |
| Friends | Friend requests (accept, decline, sent), add a friend by phone number |
| Groups | Create, add or remove members, rename, leave (optionally silently) |
| Appearance | Light, dark or system theme, wallpaper colors (Android 12+), tinted chat background, per-chat colors |

**Not supported:** group calls, mini apps, reminders, editing your profile.

## Install

1. Download `zalom3ui-release-apk` from the latest **CI** run on GitHub. It holds two APKs, one per phone
   architecture (the arm64 one is about 6 MB). Install one of them:
   - `app-arm64-v8a-release.apk`: for almost every phone. Try this one first.
   - `app-armeabi-v7a-release.apk`: for old phones that are 32-bit only, and for phones running a 32-bit Android (some
     phones with a 64-bit chip, such as a Cortex-A53, ship a 32-bit system). To tell: `adb shell getprop
     ro.product.cpu.abi` prints `arm64-v8a` or `armeabi-v7a`; on the phone, an app such as CPU-Z shows the
     architecture (`aarch64` is 64-bit, `armv7l` is 32-bit). If Android answers "App not installed" for the arm64 APK,
     install the other one. Do not use it on a 64-bit phone that can run the arm64 one: it is slower there.

   The 32-bit build has no hardware AES (the Cortex-A7 has none), so it does the message and call encryption in
   software and video calls may be slow. It builds in CI, but it has not been tried on real 32-bit hardware yet.
2. Copy the APK to the phone and open it (allow installing from that app when asked), or `adb install` it.
3. Open **Zalỏ** and log in. Android 8.0 or newer. Zalỏ has its own app id (`com.zalom3ui.app`), so it installs next to
   Zalo Lite or the official app.

### Logging in

- **Zalo is on another phone:** tap **Get QR code** and scan it with that phone (Zalo → QR icon next to search).
- **Zalo is on this same phone:** Zalo only accepts a live scan, not a saved picture of the code, and you can't scan
  your own screen. Tap **Get QR code**, then **Send to another phone**, open the code on that phone's screen (or a
  computer's) and scan it with Zalo on this one.
- Confirm on the phone.

*Settings → Restart Zalo service* restarts the core if it gets stuck; *Log out of Zalo* removes the session and the
message cache from the phone.

### Stay connected

*Settings → Stay connected* (when Zalo runs on this phone) decides when Zalỏ holds the connection to Zalo:

| Mode | What happens | Trade-off |
|---|---|---|
| **Always** (default) | Connected all the time, through the "Connected to Zalo" notification, also after a reboot | Notifications and incoming calls while the app is closed. Zalo Web/PC can't stay connected at the same time |
| **While open** | Connects when you open the app; disconnects a minute after you leave it (never during a call or an upload) | No notifications or calls while closed; missed messages arrive when you open it. Zalo Web/PC works while Zalỏ is closed |
| **While open + notification bridge** | As *While open*, but when the official Zalo app on this phone shows a message notification, Zalỏ connects for about a minute, posts its own notification and disconnects again. Needs notification access; can hide Zalo's own notification. Zalo's call notifications are never touched | Notifications without a permanent connection; calls still ring in Zalo. Each bridge connect briefly takes the one web slot, so it can disconnect Zalo Web/PC |

*While open* is only safe for your 1:1 history if Zalo replays everything you missed while disconnected, and how much
it replays is not known yet. That is why **Always** stays the default until the replay test
([docs/replay-test.md](docs/replay-test.md)) shows otherwise. *Settings → Developer → Replay log* records, for every
reconnect, how long Zalỏ was offline and what Zalo sent back (ids, counts and times only, no message text).

### Codes from Zalo's notifications

*Settings → Codes from Zalo's notifications* (off by default, any *Stay connected* mode) is for one-time codes that
services and banks send through Zalo. If Zalo does not deliver those to Zalỏ's connection, the official Zalo app on the
same phone still shows them in its notification. With this on, Zalỏ reads the notification text (in memory only; nothing
is stored or logged) and posts its own "Code 482910 · title" notification with a **Copy** action, and hides the code on
the lock screen. It needs notification access (the same switch the notification bridge uses, offered right in that
group when it is already allowed), never brings Zalỏ online, never touches Zalo's call notifications, and stays quiet
when Zalỏ already announced the same code in the last two minutes.

*Settings → Developer → Socket log* shows what Zalo sent over the connection, frame by frame (command numbers, key
names, array lengths, which messages were kept or dropped and why; never message text). It answers whether a code reaches
Zalỏ at all.

## Running Zalo somewhere else (optional)

The app can also use a Zalỏ server on another machine instead of running Zalo itself: *Settings → Zalo runs on →
Remote server*, then enter its address and token. The server is the same Rust core with an HTTP API and a small web UI.

```bash
cd rust
cargo build --release -p zalo-cli
PUBLIC_DIR=../public ./target/release/zalom3ui-server   # http://127.0.0.1:3000
```

Open the page, click **Log in with QR code** and scan it from the Zalo app.

| Variable | Default | Purpose |
|---|---|---|
| `HOST` | `127.0.0.1` | Interface to listen on |
| `PORT` | `3000` | Port |
| `DATA_DIR` | `./data` | Where the session and message cache are stored |
| `ACCESS_TOKEN` | *(none)* | Required for every API/WebSocket request when set. **Mandatory** if `HOST` is not loopback |
| `READ_RECEIPTS` | `true` | Send "seen" receipts when you read a chat, like the official app. `false` to read silently |
| `PUBLIC_DIR` | *(none)* | Serve the web UI from this folder (`public/` in this repo) |
| `LOG_FILE` | *(none)* | Also write the log to this file (rotated) |

To use it from another device, expose it with a token and open `http://<server>:3000/?token=<ACCESS_TOKEN>` once on
each device. Put it behind HTTPS (a reverse proxy, Tailscale, etc.) if it leaves your home network: anyone with the
token can read and send messages as you.

```bash
HOST=0.0.0.0 ACCESS_TOKEN=$(openssl rand -hex 24) PUBLIC_DIR=../public ./target/release/zalom3ui-server
```

The Android emulator reaches a server on your computer at `http://10.0.2.2:3000`.

## Building

You need Rust (stable) with the `aarch64-linux-android` and `armv7-linux-androideabi` targets, the Android SDK and NDK,
and JDK 17.

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi
cd android
./gradlew testDebugUnitTest assembleDebug assembleRelease
```

Gradle compiles the Rust core for the phone itself, once for 64-bit ARM and once for 32-bit ARM (task `buildRustCore*`,
using the NDK's clang), and packs each into its own APK (`app-arm64-v8a-*.apk`, `app-armeabi-v7a-*.apk`) next to the
small Opus codec (`libzaloopus.so`, C). Rust tests: `cd rust && cargo test --workspace`.

| Folder | What it is |
|---|---|
| `rust/zalo-proto` | The Zalo web protocol: login, encryption, the chat WebSocket, the HTTP API |
| `rust/zalo-media` | Call media (zrtc, SRTP, RTCP, H.264) and history-sync formats (ZDB, LZMA, SQLite reader) |
| `rust/zalo-server` | `ZaloSession`, the message store, calls and history sync, and the REST/WebSocket API |
| `rust/zalo-android` | `libzalocore.so`: the JNI bridge the app calls |
| `rust/zalo-cli` | `zalom3ui-server`, the standalone server |
| `android/` | The app (Kotlin, Jetpack Compose, Material 3 Expressive) |
| `public/` | The web UI the standalone server serves |

The Rust code is a port of an earlier TypeScript backend built on [zca-js](https://github.com/RFS-ADRENO/zca-js). That
code was removed after the port; it is still in git history (last at `c429f15`), and Rust doc comments that cite
`src/*.ts` refer to it.

## Security notes

- The saved session (`credentials.json`) is equivalent to your logged-in Zalo account. It is written with `0600`
  permissions, lives in the app's private storage on the phone, and is git-ignored on a server. Never share it.
- On the phone, nothing listens on the network: the app talks to the core in-process.
- **Log out** deletes the saved session and the message cache.

## API

The standalone server, and the core inside the app, answer the same API. All endpoints are under `/api` and accept
`Authorization: Bearer <token>` (or `?token=`) when a token is set.

| Method | Path | Description |
|---|---|---|
| `GET` | `/status` | Session state, QR image (data URL) while logging in |
| `POST` | `/login` | Start a QR login |
| `POST` | `/retry` | Retry now: a saved session waiting to log back in, or a dropped connection waiting to reconnect |
| `POST` | `/logout` | Log out and delete saved data |
| `POST` | `/reconnect` | Restart the real-time connection |
| `POST` | `/listener/pause` · `/listener/resume` | Disconnect from Zalo without logging out (state `paused`), and connect again (used by *Stay connected*) |
| `PUT` | `/notify-window` | `{ "triggerTs"?, "ttlMs"? }`: for a while, replayed messages newer than the trigger notify (the notification bridge); `DELETE` closes it |
| `GET` | `/threads` | Chats and contacts: pinned first, then most recent; unread counts, `pinned`, `mutedUntil` |
| `GET` | `/threads/:kind/:id/info` | `{lastOnline}` for users, `{members}` for groups |
| `GET` | `/threads/:kind/:id/messages?before=&limit=` | Messages (`kind` is `user` or `group`) |
| `POST` | `/threads/:kind/:id/messages` | Send `{ "text", "quoteId"?, "mentions"?: [{uid, pos, len}] }` |
| `PUT` | `/threads/:kind/:id/messages/:msgId/reaction` | `{ "reaction": "heart" }` (see `GET /reactions`), `null` to remove |
| `POST` | `/threads/:kind/:id/messages/:msgId/recall` | Recall your own message for everyone |
| `DELETE` | `/threads/:kind/:id/messages/:msgId` | Delete a message for yourself |
| `POST` | `/threads/:kind/:id/messages/:msgId/forward` | `{ "to": [{kind, id}] }` (text messages) |
| `POST` | `/threads/:kind/:id/read` · `/unread` · `/typing` | Mark read (sends seen receipts unless `READ_RECEIPTS=false`), mark unread, show "typing…" |
| `PUT` | `/threads/:kind/:id/pin` · `/mute` | `{ "pinned": true }` · `{ "duration": "1h" \| "4h" \| "8am" \| "forever" \| null }` |
| `DELETE` | `/threads/:kind/:id` | Delete the conversation |
| `POST` | `/threads/:kind/:id/attachments?name=&width=&height=` | Send one photo or file (the raw request body); photos need their pixel size |
| `POST` | `/threads/:kind/:id/voice?name=` | Send a voice message (raw body or multipart) |
| `GET` | `/search?q=&limit=` | Search message text in all chats |
| `GET` | `/stickers?q=` · `POST /threads/:kind/:id/stickers` | Sticker search · send `{ "id" }` |
| `PUT` | `/presence` | `{ "kind", "id" }` of the chat on screen (or `{}`), repeated while open, so it doesn't notify |
| `GET` | `/friends` · `/friend-requests` · `/friend-requests/sent` | Friends, received and sent requests |
| `POST` | `/friend-requests` · `/friend-requests/:userId/accept` · `/reject` | Send `{ "userId", "message"? }`, answer one; `DELETE /friend-requests/:userId` takes it back |
| `GET` | `/users/find?phone=` | `{id, name, avatar, isFriend, requested}` |
| `POST` | `/groups` | Create: `{ "name"?, "members": [userId…] }` (2 or more) |
| `POST` | `/groups/:id/members` | Add `{ "members": [userId…] }`; `DELETE /groups/:id/members/:userId` removes |
| `PUT` | `/groups/:id/name` | `{ "name" }` |
| `POST` | `/groups/:id/leave` | `{ "silent"?: true }` |
| `POST` | `/groups/:id/polls` | Create `{ "question", "options", "allowMultiple"?, "allowAddOptions"?, "anonymous"?, "hideVotePreview"?, "expiresAt"? }` |
| `GET` | `/polls/:pollId` | Poll details; `POST …/vote` `{ "optionIds" }`, `POST …/options` `{ "options" }`, `POST …/lock` |
| `POST` | `/calls/start` | `{ "userId", "video"? }` |
| `POST` | `/calls/:callId/answer` · `/decline` | Answer or decline a ringing call |
| `POST` | `/calls/hangup` · `/calls/mute` · `/calls/camera` | End the call · `{ "muted" }` · `{ "on" }` |
| `GET` | `/calls/current` | The active call, or 404 |
| `GET` | `/debug/call-log` | Call signalling log (redacted); `DELETE` clears it |
| `GET` | `/debug/replay-log` | What Zalo replayed after each reconnect (no message content); `DELETE` clears it |
| `GET` | `/debug/socket-log` | The last 500 WebSocket frames, structure only (cmd, key names, array lengths, kept or dropped messages and why; no text), newest first, plain text; `DELETE` clears it |
| `GET` · `PUT` | `/debug/replay-mode` | `{ "mode": "null" \| "newest" \| "page" }`: how the replay is requested, for the replay test |
| `POST` | `/history-sync/start` · `/cancel` | Ask the phone's Zalo app for its chat history ([docs/history-sync.md](docs/history-sync.md)) |
| `GET` | `/history-sync/status` | State, progress and a log of what the phone sent (no message content) |

`GET /ws` is a WebSocket that pushes `status`, `message` (with `notify` and `threadName` when a phone should show it),
`message_updated` (reactions, recalls, receipts, sticker images), `message_deleted`, `read`, `typing`, `threads`,
`poll`, `call`, `history_sync` and `upload_progress` events. `upload_progress` reports
`{ threadKind, threadId, uploadId, sent, total, done }` for each file being sent, at most every 100 ms. `GET
/ws/call-audio` carries a call's audio and video frames
([docs/call-audio.md](docs/call-audio.md)). Inside the app, the same events and frames come over JNI instead.
