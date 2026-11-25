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
- **Packetization:** WebRTC's own `RtpPacketizerH264` / `RtpDepacketizerH264`: RFC 6184 single NAL units, FU-A
  fragments, STAP-A aggregates. Standard.
- **FEC:** on by default (`FEC_VIDEO_ENALBE = 1`): video RTP is wrapped in **RED** (payload type **118**, RFC 2198)
  with **ULPFEC** (payload type **119**, RFC 5109) packets alongside. A receiver can unwrap RED and ignore FEC; using
  FEC only helps under loss.
- **Retransmission and key frames:** NACK on (`videoNack`), key-frame requests via RTCP (PLI/FIR:
  `OnReceivedIntraFrameRequest`), receiver bandwidth via REMB (`rembPacket`), Google congestion control (`gccMode`).
- **Size and rate** (`zrtc_config`): `maxWidth` 640, `maxHeight` 480, default encode height 360, `codecMaxFps` 24,
  `minBitrate` 150 / `maxBitrate` 800 kbit/s, `startupBitrate` 200, and a ladder in `bwProfiles` (240p 8 fps 30 kbit/s
  … 720p 24 fps 1100 kbit/s).
- **Control:** camera on/off and quality negotiation go through zrtc forward messages (command 32, sub-type 10,
  `initZRTPPacketVideoControl`) and `partnerCameraOff` events.
- **Call type:** `extendData.callType` 1 for video (0 voice); declining a video call sends call type 3.

The call log now describes any video that arrives (`call_done` → `stats.video`: payload types, the type inside RED,
H.264 NAL unit types), so answering one video call confirms the RED/H.264 layout before any of the code below.

## Plan

| Step | Where | What |
|---|---|---|
| V0 | — | Answer a video call (voice only for now) and share the call log: confirms RED 118 / H.264 97, SSRCs, NAL types. |
| V1 | server | Receive: kind 14 → SRTP → RED unwrap → RFC 6184 depacketizer → whole access units (Annex B, timestamp, key-frame flag) to the app over the call socket (new message type `03`). Send RTCP PLI when video starts or a frame is lost, NACK for missing sequence numbers. |
| V2 | app | Decode with `MediaCodec` (`video/avc`, hardware) onto a `SurfaceView`; the call screen shows the other side full screen. |
| V3 | app | Camera: CameraX/Camera2 into a `MediaCodec` H.264 encoder (Surface input, constrained baseline, 640×480 or 360p, 15–24 fps, 300–800 kbit/s, key frame every ~2 s and on request); access units to the server (`04`); small self-view. |
| V4 | server | Send: RFC 6184 packetizer (FU-A, STAP-A for SPS/PPS + IDR), RED wrapping (FEC packets optional), SRTP, kind 13; video sender reports; bitrate follows the other side's REMB / loss. |
| V5 | both | Answer video calls with video (our `extendData.video` codec list, `callType` 1), camera on/off, front/back switch, speaker on by default. |
| V6 | both | Outgoing calls, voice and video: `requestcall` → servers, session, config → init call as caller (command 11) → `request` with our media server → wait for `answer`. |

Steps V1–V2 (see the other side) need no camera permission and can be tested against any video call; V3–V4 need a
real phone to check the other side sees us.
