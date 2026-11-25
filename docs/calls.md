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

Zalo Web (chat.zalo.me) has no call engine at all: it ships the same `voicecall` request builders but never asks for
the microphone (no `getUserMedia`), and Zalo PC's only `RTCPeerConnection` reads the local IP. So there is no
standard-WebRTC path to fall back on; every 1:1 call goes through zrtc.

What the call program needs from signalling (`zrtc::CallController` setters): the call id, the partner's id, a
**session id**, a **token**, an **SRTP key** (`setSrtpKey`), the protocol / packet mode, and the media server
(`setCurrentServer(ZRTPServerInfo)`); `_parseExtendData` reads the request's `extendData`. The keys come from Zalo's
servers in the call request rather than from a handshake between the phones, so there is no key agreement to rebuild,
only the packet format and the start-up exchange with the media server (`_sendRequestInitZRTPToServer`, ping, echo,
P2P binding, TCP fallback).

### zrtc packets (mapped)

Worked out from Zalo PC's call program (arm64) and checked by running its own packet functions in an emulator:
`src/zrtc.ts` builds and parses them, and `test/zrtc.test.ts` holds byte strings that the official functions
produced. Numbers are little-endian; strings carry a 16-bit length (some forwards use 32-bit).

Byte 0 is the kind:

| Kind | From | Layout |
|---|---|---|
| 1 request | client (and server, for forward/echo) | 21-byte control header + body |
| 2 response | media server | 21-byte control header + body |
| 3 audio | client | `03`, u32 stream token, SRTP-protected RTP |
| 4 / 5 audio | server | `04` + RTP, or `05`, u32 token, RTP |
| 7 audio FEC | both | client: `07` + token + data; server: `07` + data |
| 13 / 14 / 15 video | client / server / server | like audio |
| 0x7f P2P | other phone | `7f`, u8, u8, u32 sender, u16 command, … (binding, echo, direct media) |

Control header: kind (u8), flag (u8: `7e` on init/end call, `01` on the rest), u32, u32, **u32 own zrtc user id**,
u32 (server id on ping and end call), **u16 command**, u8 sub-value. Payloads stop at 1400 bytes.

| Command | Sent by | Body |
|---|---|---|
| 11 init call (caller) | client | u32 call id, u32 partner id, str16 session; sub = attempt (1, then 2) |
| 12 init call (person called) | client | same + str16 extra (empty at first) |
| 11/12 response | server | u32 error; if 0: u32, u32, str16 our address as the server sees it (for P2P) |
| 2 ping | client | str16 session; user id + server id in the header |
| 3 end call | client | u32 call id; response: u32 error |
| 14 change address | client | str16; response: u32 error |
| 5 echo | both | str16 |
| 32 forward | both | sub-value picks str32 or str16 |

A real incoming request (`voip` / `request`): `uidFrom` and `uidTo` are the 32-bit zrtc ids of caller and callee,
`uidN` the caller's noised Zalo id, `callId` 32-bit, `session` a ~150-character base64url string, `rtpAddress` /
`rtcpAddress` one media server (`ip:4200`), `codec` the caller's list (Opus 16 kHz mono, payload type 112, 20 ms
frames), and `params` with `extendData` (call type, `packetMode: 2`, the caller's P2P candidates, `serverAddr` with
IPv4/IPv6 media servers), `zrtc_config` (`srtpMode: 1`, no `srtpKey`, audio 16 kHz mono 20 kbit/s, packets up to
1300 bytes) and `settings`. **SRTP key:** with no `srtpKey`, `CallController::parseCallConfig` takes the first
`GetSrtpCryptoLengths(srtpMode)` = 30 characters of the session as the master key (16) + salt (14); mode 1 has a
10-byte tag: AES_CM_128_HMAC_SHA1_80.

What a call needs, all from Zalo's servers rather than from a handshake between the phones: the 32-bit zrtc ids
(own, partner, call), the **session**, the media **servers** (`rtpAddress`/`rtcpAddress`, `servers`, `tcpServers`,
`fallbackServers`, IPv6 variants), and the **SRTP key**. Zalo PC passes `srtpKey` unchanged to libsrtp as the
master key + salt, the same key for both directions (each stream is told apart by its SSRC), with `srtpMode` as the
crypto suite (1 or 2: AES-128 counter mode with an HMAC-SHA1 tag). That is plain RFC 3711 SRTP. 

Where these sit in an incoming call (`voip` / `request` control, read by `_parseActRequest`): `status`, `uidFrom`,
`uidTo`, `ts`, `uidN` and `params`, which holds `platform`, `protocol`, `id`, `sessId`, `extendData`, `rtpIP`,
`rtcpIP`, `avatar`, `Dname`, `codec`, `servers` and **`zrtc_config`**. `zrtc_config` (`ZRtcConfig::fromJson`) is
the media setup: `srtpKey`, `srtpMode`, `srtpNoSeq`, `srtpRTCP`, `packetMode`, `opus` / `g711` / `pcma` / `pcmu`,
`audioSampleRate`, `audioChannel`, `audioBitrate`, `audioNack`, `jbMs`, `tcpServers`, `fallbackServers`,
`selectedServer`, `numSelectedServer`, `supportTCP`, `forceTCP`, `preferIpv6`, `partnerIP`, `sP2P` and about two
hundred tuning values (echo cancellation, gain control, bandwidth estimation, video). The caller gets the same
`servers` / `zrtc_config` / `sessId` in the answer to its `requestcall` (`_parse401Response`).

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
  announced (replayed history).
- **Decline:** the call notification has **Decline**. Zalo PC's call program declines (or closes the incoming call
  window) with signal 402 and **status 3** (`ZCallInfo::UiAnswerCallEvent`, `UiCloseEvent`); accepting is status 0
  (`_trySendAcceptSignal`), busy status 1. The renderer sends it as `/api/voicecall/answer`: the caller's noised id
  (`uidN`) as `callerId`, the call id as a number, the status, the codec list, `rtcpAddress` / `rtpAddress` (the call's
  media server), `session_id`, and, because the call program always hands over its `extendData`
  (`CallController::getExtendData`: call type, platform 2 for PC, packet/SRTP/congestion modes, P2P candidates, media
  servers), `extendData` with `subCommand: 3`. Without those two, Zalo ended the call (accepting it on the phone
  afterwards said "call ended") but neither the caller nor the other devices heard about it. **Works:** the caller's phone shows
  "Người nhận từ chối cuộc gọi" (the recipient declined the call), as when declining in the official app. Declining in the official app gives the same: the other devices get an `answer`
  control with status 3. Two earlier tries used `/api/voicecall/cancel` (the caller giving up): Zalo accepted it but
  the caller kept ringing. Zalỏ also sends `ringring` first, as Zalo PC does when a call arrives. The call log records
  `ringring`, then `declined` with Zalo's answer, or `decline_failed`.
- **Answer:** the incoming call is a proper call notification (`CallStyle`, full screen on the lock screen) with
  Answer and Decline. Answer asks for the microphone, opens the call screen and picks up the way Zalo PC does:
  `ringring`, join the call's media server over UDP (init call, command 12, with the request's `uidTo` / `uidFrom` /
  `callId` / `session`; the reply's second number is the **token** every audio packet carries and the server id pings
  carry), then `/api/voicecall/answer` with status 0, codec Opus 16 kHz mono (payload 112), that media server, our
  `extendData` and `subCommand: 3`. Audio then flows as described in `docs/call-audio.md`: the server decrypts the
  caller's SRTP (key = the session's first 30 characters; `src/srtp.ts`, checked against libsrtp) and hands the Opus
  frames to the app, which plays them and sends the microphone back (silence fills any gap); pings every 2 s; Hang up
  sends `/api/voicecall/endcall` and the zrtc end call. The call log records the media server's replies and a
  `call_done` line with what the audio looked like: packet counts per kind, how many decrypted, RTP payload types,
  timestamp steps and the first byte of a few Opus frames — never the audio. (`POST /api/calls/:id/answer-test` still
  picks up without the app's audio for 30 s, for testing.)
- **Control poll:** only when the account's settings say Zalo PC would long-poll (`enable_ctrl_socket` off or
  `disable_lp` off). On an account with both set the chat socket carried every call, and the poll answers kept
  repeating the same backlog; repeats now make the poll wait 30 s. An incoming call shows a "📞 Bảo is calling" notification with **Open Zalo**;
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
