# Call audio on Android — contracts between the pieces

Zalỏ's core (Rust, `libzalocore.so`, running inside the app's process) owns the call: Zalo's signalling, the UDP connection to Zalo's media
server (zrtc), SRTP. It hands **Opus frames** to the app and takes Opus frames from it. The app (Kotlin) owns the
microphone, the speaker, the Opus codec and the call screen. See `docs/calls.md` for the protocol itself.

```
 mic ─► AudioRecord ─► OpusEncoder ─► CallAudioLink ──link──► core ─► SRTP ─► zrtc UDP ─► Zalo
 speaker ◄─ AudioTrack ◄─ OpusDecoder ◄─ JitterBuffer ◄─ CallAudioLink ◄──link── core ◄─ … ◄─ Zalo
```

Audio format everywhere: **Opus, 16 kHz, mono, 20 ms frames (320 samples), ~20 kbit/s**, RTP payload type 112.

## 1. Opus codec (`com.zalom3ui.app.call`, file `Opus.kt`, native `libzaloopus.so`)

libopus built from source with CMake (FetchContent of the official release tarball, pinned by SHA-256), wrapped
with JNI.

```kotlin
class OpusEncoder(sampleRate: Int = 16000, channels: Int = 1, bitrate: Int = 20_000) : AutoCloseable {
    /** One 20 ms frame: sampleRate / 50 * channels samples. Returns the Opus packet. */
    fun encode(pcm: ShortArray): ByteArray
}

class OpusDecoder(sampleRate: Int = 16000, channels: Int = 1) : AutoCloseable {
    /** Decodes one packet into one 20 ms frame; null = the packet was lost (packet-loss concealment). */
    fun decode(packet: ByteArray?): ShortArray
}
```

Encoder: `OPUS_APPLICATION_VOIP`, in-band FEC on, expected packet loss 10 %, DTX off, complexity 5.

## 2. Audio engine (`com.zalom3ui.app.call`)

```kotlin
interface FrameCodec : AutoCloseable {
    fun encode(pcm: ShortArray): ByteArray
    fun decode(packet: ByteArray?): ShortArray
}

class JitterBuffer(targetFrames: Int = 3, maxFrames: Int = 25) {
    fun push(seq: Int, packet: ByteArray)   // seq: RTP sequence number, 16-bit, wraps
    fun pop(): ByteArray?                   // next frame in order, or null (lost / not arrived yet) once playing
}

class CallAudioEngine(context: Context, codec: FrameCodec, send: (ByteArray) -> Unit) {
    fun start()
    fun stop()
    fun onPacket(seq: Int, timestamp: Long, opus: ByteArray)  // from the network
    var muted: Boolean
    var speakerOn: Boolean
}
```

- Capture: `AudioRecord(VOICE_COMMUNICATION, 16000, mono, PCM16)`, platform `AcousticEchoCanceler`,
  `NoiseSuppressor`, `AutomaticGainControl` when available; 320-sample reads → encode → `send` (silence frames
  while muted).
- Playback: every 20 ms pop the jitter buffer → decode (null → concealment) → `AudioTrack(USAGE_VOICE_COMMUNICATION,
  CONTENT_TYPE_SPEECH)`. `AudioManager.MODE_IN_COMMUNICATION` and audio focus while running; speaker via
  `setCommunicationDevice` (API 31+) or `isSpeakerphoneOn`.

## 3. Link to the core (`CallAudioLink`)

When Zalo runs on the phone, `CallAudioLink` uses `NativeAudioTransport`: an in-process handle from `ZaloCore`
(`audioOpen` / `audioSend` / `audioNext` / `audioClose`, see `rust/zalo-android/src/lib.rs`). With a server elsewhere it
uses the WebSocket `ws(s)://<server>/ws/call-audio?token=<token>` instead. Both carry the same binary messages:

| Direction | Bytes |
|---|---|
| server → app | `01`, u16 RTP sequence (big-endian), u32 RTP timestamp (big-endian), Opus packet |
| app → server | `02`, Opus packet (one 20 ms frame) |

The server sends the other side's audio already decrypted (just the Opus payload). The socket is open only while a
call is connected; the server closes it when the call ends.

Video uses the same socket (docs/video-calls.md):

| Direction | Bytes |
|---|---|
| server → app | `03`, u8 flags (bit 0: key frame), u32 RTP timestamp (90 kHz, big-endian), H.264 access unit in Annex B (start codes; SPS/PPS in front of every key frame) |
| app → server | `04`, u8 flags (bit 0: key frame), H.264 access unit in Annex B (SPS/PPS in front of every key frame) |
| server → app | `05`: the other side asked for a key frame (RTCP PLI / FIR): encode one now |
| app → server | `06`: our decoder needs a key frame (start, or a decode error): the server sends a PLI |

## 4. Call API (server, under `/api`)

| Request | Does |
|---|---|
| `POST /calls/:callId/answer` | Picks up a ringing call: joins the media server, answers. 204, or 4xx/5xx `{error}` |
| `POST /calls/:callId/decline` | Declines (exists) |
| `POST /calls/start` | `{userId, video}`: places a 1:1 call (see below). Returns the call (`state: "calling"`, `outgoing: true`); 4xx/5xx `{error}` |
| `POST /calls/hangup` | Ends the call in progress: hang up when connected, cancel while calling / ringing. 204 |
| `GET /calls/current` | `{callId, callerId, callerName, video, state: "connecting" \| "calling" \| "ringing" \| "connected", connectedAt, outgoing?: true}` or 404. For a call we placed, `callerId` / `callerName` are the person called and `state` starts as "calling" |
| `POST /calls/mute` | `{muted}`: tells the other side (the app silences the microphone itself) |
| `POST /calls/camera` | `{on}`: tells the other side our camera is on / off (`/api/voicecall/conf`); the app starts / stops sending video |

Events on the existing `/ws` socket, `{"type":"call","call":{…}}`:

- `{"type":"incoming","call":{callId, callerId, callerName, video, group, groupId?, ts}}` (exists)
- `{"type":"connected","callId","callerId","callerName","video","connectedAt"}` (new)
- `{"type":"ended","callId","act"}` (exists; `act` says why: `"endcall"`, `"cancel"`, `"declined"`, `"hangup"`…). For
  calls we placed: `"declined"` (they declined), `"busy"`, `"no_answer"` (rang 60 s), `"unreachable"` (no ring within
  20 s, or Zalo said they can't be reached), `"cancel"` (we cancelled), `"failed"` (Zalo or the media server refused),
  plus `"endcall"` / `"end_call"` when they hang up.
- Calls we placed (`POST /calls/start`), each with the call's fields (`callId, callerId, callerName, video, state,
  connectedAt, outgoing: true`):
  `{"type":"outgoing",…}` (request sent, `state: "calling"`), `{"type":"ringing",…}` (the other phone rings), then
  `{"type":"connected",…}` or `{"type":"ended",…}`.
- `{"type":"peer","callId","cameraOn","muted"}` (new): the other side turned its camera / microphone on or off
  (`voip` controls `onoff_camera`, `params.type` 0 = off, and `mute_audio`, 1 = muted)

Placing a call (`ZaloSession.startCall`, mirroring the Zalos project's engine): `requestcall` (answer: media `servers`,
`sessId`, `zrtc_config`, our 32-bit `fromId` and the callee's `toId`; its field names go to the call log) -> join the
first media server that accepts us as the caller (init call command 11) -> `request` naming that server, Opus codec
and our extendData (`callType` 1 for video) -> wait for the callee's controls: `ring_ring`, `answer` (status 0 = picked
up: we send `answerack`, start silence fill and the audio / video link; 3 = declined, 1 = busy), `busy`, `reject`,
`cancel`. Cancelling while it rings sends `cancel`; hanging up when connected sends `endcall`. After `connected`
everything (the `/ws/call-audio` socket, mute, camera, hang up) works as for an answered call. For a video call the
app turns its camera on after `connected` with `POST /calls/camera`.

## 5. Screens (`com.zalom3ui.app.ui.call`)

- Incoming call: `NotificationCompat.CallStyle.forIncomingCall` with Answer / Decline and a full-screen intent to the
  call screen (ringing state) when the phone is locked. It rings with the ringtone on repeat (`FLAG_INSISTENT`, channel
  `calls_ringing`) until it is answered, declined or over. The notifier also opens the ringing call screen itself
  (`CallPopup`), so the call pops up while the device is in use; Android allows that when Zalỏ is visible or may
  "Display over other apps" (Xiaomi adds its own background pop-up switch). Settings → Incoming calls links to each
  switch. No pop-up while another call is in progress or once the call was answered from the notification.
- Call screen (Compose, the app's own theme): avatar, name, "Connecting…" / timer, Mute, Speaker, Hang up.
- `CallService`: foreground service, type `microphone`, in the main process; owns `CallAudioEngine` +
  `CallAudioLink` while a call is connected; its notification is `CallStyle.forOngoingCall` with Hang up.
- Permissions: `RECORD_AUDIO` (asked when answering), `FOREGROUND_SERVICE_MICROPHONE`, `USE_FULL_SCREEN_INTENT`,
  `MODIFY_AUDIO_SETTINGS`.
