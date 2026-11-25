# Calls in Zalỏ — research notes and plan

Zalỏ can't place or answer voice/video calls yet. This page records what is known about how Zalo calls work, from
reading the official Zalo PC 26.10.20 client and [zalo-swift-re](https://github.com/qnguyen3/zalo-swift-re) (Olaz), and
the plan to get there. Nothing from VNG is copied into this repo.

## How Zalo PC (and Olaz) does calls

Two official pieces work together:

1. **The renderer** (Zalo PC's web code, `pc-dist`): a call helper module relays call messages between Zalo's servers
   and the native call program. It never touches audio or video.
2. **`ZaloCall`**, a native Qt program inside Zalo PC (`ZaloHelper.app` on macOS). It runs the call itself: microphone,
   camera, the call window, and the media connection to Zalo's servers.

They talk over two local sockets. Each frame is `hex(AES-128-CBC(json))` ended by `$`, with a fixed key shipped in
Zalo PC (Olaz documents it in `Sources/Olaz/ZaloRuntime/CallTransport.swift`). Olaz's own code only starts `ZaloCall` and
relays these frames; that is why calls work there and why that approach can't move to Android (there is no `ZaloCall`
for Android).

## Signalling: ordinary Zalo web requests

Ringing, answering and hanging up are HTTP requests to Zalo's API, the same kind `zca-js` already makes, under the
"voice call" domain:

| Command id | Endpoint |
|---|---|
| 11300 | `/api/voicecall/requestcall` |
| 11301 | `/api/voicecall/request` |
| 11302 | `/api/voicecall/ringring` |
| 11303 | `/api/voicecall/answer` |
| 11304 | `/api/voicecall/answerack` |
| 11305 | `/api/voicecall/cancel` |
| 11306 | `/api/voicecall/endcall` |
| 11307–11310 | `holdreq`, `holdresp`, `unholdreq`, `unholdresp` |
| | also `conf`, `logendcall`, `requestsharescreen`, and `group/*` (request, requestcall, ringring, answer, join, adduser, cancel, endcall, finish, ping, state, notify, broadcast, sharescreen) |

Each request's parameters (from the official client):

| Request | Parameters |
|---|---|
| `requestcall` | `calleeId`, `callId`, `codec` (JSON list text), `typeRequest`, `imei` |
| `request` | `calleeId`, `rtcpAddress`, `rtpAddress`, `codec`, `session`, `callId`, `imei` (+ `extendData`, `subCommand: 3`) |
| `ringring` | `callerId`, `callId`, `status: 0`, `imei` |
| `answer` | `callerId`, `callId`, `status`, `codec`, `rtcpAddress`, `rtpAddress`, `session_id` |
| `answerack` | `calleeId`, `callId`, `status: 0`, `imei` |
| `cancel` | `callerId`, `callId`, `callType`, `status: 0`, `imei` |
| `endcall` | `uidTo`, `callId`, `status: 3`, `imei` |

All are GET requests with the parameters AES-encrypted the way zca-js already does for other Zalo APIs.

`ZaloCall` decides *what* to send: it hands the renderer a numbered signal and the renderer turns it into one of these
requests. For example signal 401 becomes a call request with `calleeId`, `callId`, `codec` and call `type`; signal 416
carries the media addresses (`rtcpAddress`, `rtpAddress`, …). Incoming call events arrive on the realtime connection
(the WebSocket `zca-js` listens on) and are passed back to `ZaloCall`.

So the signalling can be reproduced in Zalỏ's server once the exact parameters and answers are known.

## Media: VNG's own engine ("zrtc")

`ZaloCall` (36 MB, macOS universal) is built on VNG's **zrtc** engine:

- It contains Google's WebRTC media code: Opus audio, VP8 and H.264 video, SRTP, congestion control.
- Its network layer is VNG's own. There is no SDP, ICE, STUN/TURN or DTLS, so a standard WebRTC peer connection can't
  talk to it. Its symbols name its own packet handling (`ZRtc` packets, a network worker) and a JSON message layer
  (`zrtc_call::ZMessageDeserializer`).
- The binary keeps its C++ symbol names, which makes static analysis practical.

Getting audio to flow means reimplementing that network layer (how a call connects to Zalo's media relays, the packet
format, how the SRTP keys are agreed) and feeding it from an Android audio pipeline (Opus, echo cancellation).

## What Zalỏ does already

- **Who is calling:** Zalỏ's server reads the realtime `voip` controls itself (`src/calls.ts`: frame header, AES-GCM +
  zlib/gzip payload, big ids kept as text), from two places:
  - the chat WebSocket zca-js opens (`zpw_ws`). In the first test call it carried nothing about the call;
  - Zalo's **control channel**, which Zalo PC keeps apart from chat (`src/ctrlpoll.ts`). Zalo PC reads it over a
    socket or, with `enable_ctrl_socket` off, by long polling the control servers of the login answer
    (`zpw_ctrs_new`): `GET <server>/?zpw_ver=…&zpw_type=…&params=<lastActionId>,<lastActionIdOther>,<ids>&ts=<unix>`,
    held up to `zpw_conf.lp_ctl_timeout` seconds (30), answered `{error_code, data}` with `data` AES-CBC encrypted
    under the session key. Zalỏ polls the same way. On error `-69` or a kick-out it stops (Zalo PC would log out;
    Zalỏ never does), and if Zalo closes the chat socket as a duplicate while polling, polling stays off until the
    next login.

  Each control is announced once even if both channels carry it, and a call request over 2 minutes old is not
  announced (replayed history). An incoming call shows a "📞 Bảo is calling" notification with **Open Zalo**;
  it goes away when the call is answered elsewhere, cancelled or ended.
- **Call log:** every call event is appended to `call-trace.jsonl` in the server's data folder (2 MB, then rotated).
  *Settings → Call research → Share call log* exports it; *Clear call log* empties it. It holds call ids, user ids and
  Zalo server addresses, never message text.

### Recording calls without a Mac

1. In Zalỏ (account A): *Settings → Call research → Clear call log*.
2. From the official Zalo app on another phone (account B), call account A: a voice call, then a video call. Let one
   ring out, decline one in the official app on A's phone if it is there too, and answer one there and talk ~10 s.
3. *Share call log* and send the file. Its first lines say whether the control poll could start: `ctrl_setup`
   (which login fields and control servers there were), then `ctrl_poll_start` and one `ctrl_poll` per answer
   (field names and control types only), or `ctrl_poll_error` / `ctrl_poll_refused` / `ctrl_poll_off`.

That shows the caller's real call request (media addresses, codec, session) and the answer / cancel / end events, which
step 1 below needs.

## Plan

| Step | What | Needs |
|---|---|---|
| 0. Traces | Record real calls between two accounts: the call events (Zalỏ's call log, above), later the `/api/voicecall/*` answers and the UDP traffic | Two accounts; no Mac needed |
| 1. Signalling | Zalỏ's server places and receives call requests: an incoming call rings in Zalỏ with **Decline**; an outgoing call rings the other phone and can be cancelled. No audio yet | The traces from step 0; two accounts to test |
| 2. Voice | Reimplement the zrtc media connection for audio; Android side records and plays through Opus with echo cancellation | Static analysis of `ZaloCall` plus traces; many test calls |
| 3. Video | Camera capture and VP8/H.264 on the same connection | Step 2 working |

Step 1 is days of work; steps 2 and 3 are weeks, and only as fast as test calls can be made and logged.
