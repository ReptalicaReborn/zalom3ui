# Video calls — what's known and the plan

Voice calls come first (`docs/calls.md`, `docs/call-audio.md`). Video rides the same connection to Zalo's media
server, so most of the work is codecs and screens, not protocol.

## What Zalo PC's call program says about video

From the arm64 `ZaloCall` (symbols, its own packet builders run in an emulator, and its constants):

- **Packets:** video RTP goes to the media server as kind **13** (`0d`, u32 token, SRTP RTP) and comes back as kind
  **14** (`0e` + SRTP RTP); video RTCP is kind **15** both ways (token + plain RTCP). Peer-to-peer variants use P2P
  commands 8 / 9. Same token as audio.
- **Encryption:** the same SRTP context as audio (key = the session's first 30 characters); video has its own SSRC,
  which is all SRTP needs to tell the streams apart.
- **Codec:** H.264, payload type **97** (`extendData.video.codec: [{"name":"h264","payload":97}]`). HEVC exists
  (payload types 121–123) but only when both sides say `supportHevcDecode: 1`; Zalỏ says 0, so it gets H.264.
- **Packetization (1:1 calls): not RFC 6184.** The Zalos project's engine (see below), which makes working video
  calls, shows Zalo's own format: payload type **98** for a key frame and **97** for the others; the payload is the
  whole access unit in Annex B form (start codes, SPS/PPS before an IDR); frames up to 1068 bytes go in one packet,
  larger ones are cut into 1068-byte chunks each prefixed `1c` + flag (`80` first, `40` last, `00` middle); the RTP
  marker bit ends a frame; all packets of a frame share its timestamp. No RED in 1:1 calls (FEC / RED / ULPFEC
  payload types 118 / 119 exist in the call program; `fecTP: 0` turns them off).
- **Retransmission and key frames:** RTCP NACK (`videoNack`), PLI / FIR for key frames, and transport-wide congestion
  control feedback (TWCC) — Zalos sends all of these.
- **Size and rate** (`zrtc_config`): `maxWidth` 640, `maxHeight` 480, default encode height 360, `codecMaxFps` 24,
  `minBitrate` 150 / `maxBitrate` 800 kbit/s, `startupBitrate` 200, and a ladder in `bwProfiles` (240p 8 fps 30 kbit/s
  … 720p 24 fps 1100 kbit/s).
- **Control:** camera and microphone state go to the other side as `/api/voicecall/conf`
  (`calleeId`, `callId`, `isOnCam` 1/0/-1, `isOnMic` 1 muted / 0 / -1, `data: ""`); Zalỏ sends "camera off" right
  after answering a video call, and the mute state when it changes.
- **Call type:** `extendData.callType` 1 for video (0 voice); declining a video call sends call type 3.

The call log describes any video that arrives (`call_done` → `stats.video`: payload types and first-byte types).

## Prior work: Zalos

[Zalos](https://github.com/cpfriend1721994/zalos) (MIT; its call engine `engines/zcall-cpp` is BSD-3-Clause,
© doandat943 and Việt Nam Linux Family) is a native Linux Zalo client whose 1:1 voice and video calls work. Its engine
reimplements the same protocol and confirms this page and `docs/calls.md`: the callee `extendData` (now matched field
for field), `/api/voicecall/conf` for camera/mic state, the video packet format above, RTCP (reports, NACK, PLI,
TWCC). Zalỏ's code is its own (TypeScript / Kotlin), written against these notes; credit for the video format and the
`conf` request goes to them.

Answering a video call (voice only) first crashed the caller's official Android app: our `extendData` said call
type 1 but had no `video` codec list, which both official apps always send, voice calls included. It now always lists
H.264 (payload 97).

## Plan

| Step | Where | What |
|---|---|---|
| V0 | — | Answer a video call (voice only for now) and share the call log: confirms payload types 97/98, SSRCs, first-byte types. |
| V1 | server | Receive: kind 14 → SRTP → Zalo's frame assembler (pt 97/98, `1c` chunks, marker, reorder) → whole access units (Annex B, timestamp, key-frame flag) to the app over the call socket (new message type `03`). RTCP PLI when video starts or a frame is lost, NACK for missing sequence numbers. |
| V2 | app | Decode with `MediaCodec` (`video/avc`, hardware) onto a `SurfaceView`; the call screen shows the other side full screen. |
| V3 | app | Camera: CameraX/Camera2 into a `MediaCodec` H.264 encoder (Surface input, constrained baseline, 640×480 or 360p, 15–24 fps, 300–800 kbit/s, key frame every ~2 s and on request); access units to the server (`04`); small self-view. |
| V4 | server | Send: Zalo's packetizer (whole AU or 1068-byte `1c` chunks, pt 98 key / 97 delta, marker on the last, 90 kHz timestamps), SRTP, kind 13; video sender reports; answer PLI / FIR with a key frame; bitrate follows loss. |
| V5 | both | Answer video calls with video (our `extendData.video` codec list, `callType` 1), camera on/off, front/back switch, speaker on by default. |
| V6 | both | Outgoing calls, voice and video: `requestcall` → servers, session, config → init call as caller (command 11) → `request` with our media server → wait for `answer`. |

**Status:** V1–V5 are built (server: `src/video.ts`, `src/zrtcclient.ts`; app: `call/VideoDecoder.kt`,
`call/CameraVideoSender.kt`, `ui/call/VideoCallLayout.kt`) and tested against fakes; not yet on a real call. Each stream
has its own SRTP context (Zalo's apps send audio and video under one SSRC, their user id; ours is ours). V6
(outgoing calls) is next.

Steps V1–V2 (see the other side) need no camera permission and can be tested against any video call; V3–V4 need a
real phone to check the other side sees us.

## Server side, as built (V1 and V4)

`src/video.ts` (frame assembler, packetizer), `MediaCall` in `src/zrtcclient.ts` (`onVideo`, `onKeyFrameRequest`,
`sendVideoFrame`, `requestKeyFrame`), the socket messages in `docs/call-audio.md` §3. Things the app should know:

- Frames to the app carry the other side's raw 90 kHz RTP timestamp (it starts at a random value: use differences).
  Key frames always start with SPS + PPS (cached ones are put in front when the sender leaves them out).
- After a lost frame the server delivers nothing until the next key frame and asks for one itself (PLI); delta frames
  before the first key frame are dropped the same way.
- The app's frames are forwarded only after `POST /calls/camera {on: true}`; the server stamps its own 90 kHz clock on
  each frame as it arrives, so the app need not send timestamps. Send SPS + PPS in front of every key frame.
- `05` (key frame requested) can come several times a second; the server already spaces them ≥ 250 ms.
