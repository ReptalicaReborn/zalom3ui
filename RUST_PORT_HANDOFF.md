# Rust Backend Port — Handoff Guide (Split A)

This branch (`feat/rust-backend`) completes **Stage 1** of replacing the embedded Node.js (`nodejs-mobile` / `zca-js`) runtime with a pure-Rust core (`libzalocore.so` on Android + `zalom3ui-server` CLI binary).

**What is ALREADY DONE & VERIFIED on `feat/rust-backend`:**
- `rust/zalo-proto` — Pure-Rust port of the used `zca-js` v2.2.0 surface:
  - `ParamsEncryptor` (pre-login AES-256-CBC + `signkey`), session AES-128/192/256-CBC, 16-byte-IV AES-GCM WS frame decryption (`typenum::U16`), BigInt-safe JSON parser.
  - `tough-cookie`-compatible `ZaloCookieJar` (`credentials.json` carries over without re-login).
  - Full QR login (`login_qr`) & cookie login (`login_cookie`).
  - Unified WebSocket listener (`ZaloListener`) handling `msgs`, `groupMsgs`, **`pageMsgs` (OA/ZNS/OTP)**, offline push frames (`504, 513, 514, 524, 534, 551, 570`), `file_done` upload callbacks, `voip` controls, and `history_sync` controls.
  - All **44** Zalo HTTP API methods + **`VoiceCallApi`** (`src/api/voicecall.rs`), **`ControlPoller`** (`src/api/ctrlpoll.rs`), and **`get_file_service` / `get_znoise` / `download_backup`** (`src/api/mod.rs`) — **already implemented and ready for Track 1 & Track 2!**
- `rust/zalo-server` — Byte-compatible `ChatStore` (`data/messages/meta.json` + `threads/<hex>.json` + legacy `messages.json` migration, OA/ZNS/OTP parsing, Vietnamese diacritic-insensitive search), `ZaloSession`, security middleware, all messaging/friend/group/poll/sticker/presence REST endpoints, `/ws` event broadcast, and `CallTrace`.
- `rust/zalo-cli` — Standalone desktop/VPS server binary (`zalom3ui-server`) serving `public/`.
- `rust/zalo-android` + `android/` + `.github/workflows/ci.yml` — Android builds and packages `libzalocore.so` + `libzaloopus.so` (pure C), removing `libnode.so`, `libc++_shared.so`, `libnodebridge.so`, and `assets/server/server.mjs`.

> Legacy TypeScript `src/` and `test/` (including `test/fixtures/*`) are intentionally kept on `feat/rust-backend` as a byte-exact reference for Track 1 and Track 2, and can be removed in the final cleanup commit once both tracks merge.

---

## Clean Parallel Handoff (Zero Merge Conflicts)

Track 1 (**Calls**) and Track 2 (**History Sync**) are strictly isolated into separate directories and route modules:

| Track | Branch (off `feat/rust-backend`) | Files Owned (DO NOT edit the other track's files!) |
|---|---|---|
| **Track 1: Calls** | `feat/rust-calls` | `rust/zalo-media/src/call/**`, `rust/zalo-server/src/routes/calls.rs` (plus `rust/zalo-server/src/calls_manager.rs` if you want a dedicated state machine module) |
| **Track 2: History Sync** | `feat/rust-history-sync` | `rust/zalo-media/src/sync/**`, `rust/zalo-server/src/routes/history_sync.rs` (plus `rust/zalo-server/src/history_sync_manager.rs` if you want a dedicated state machine module) |

### Hooks Already Exposed by `ZaloSession` & `AppState`
Both tracks have everything they need without editing `session.rs` or `http.rs`:
- `state.session.client() -> Result<Arc<ZaloClient>, HttpError>` (authenticated `ZaloClient` with `VoiceCallApi`, `ControlPoller`, `get_file_service`, `get_znoise`, `download_backup`)
- `state.session.store() -> &ChatStore` (with `add_history_message`, `migrate_plain_thread`, `settle_history`, `unnamed_threads`, `upsert_thread`, etc.)
- `state.session.call_trace() -> &CallTrace`
- `state.session.broadcast_event(serde_json::Value)` (broadcasts `{ "type": "call", ... }`, `{ "type": "history_sync", ... }`, `{ "type": "threads" }` to all `/ws` clients)
- `state.session.subscribe_voip_controls() -> broadcast::Receiver<VoipControlEvent>`
- `state.session.subscribe_history_sync_controls() -> broadcast::Receiver<HistorySyncControlEvent>`
- `state.session.get_status() -> SessionStatus`

> Note: If either track needs one extra crate in `rust/zalo-media/Cargo.toml` (e.g. Track 2 needs `rsa` and `lzma-rs`), add it directly to `rust/zalo-media/Cargo.toml` (only Track 2 needs those, so still zero `Cargo.toml` conflict with Track 1!).

---

## Track 1: Calls (`feat/rust-calls`)

### Reference Files to Port
- `src/zrtc.ts` → `rust/zalo-media/src/call/zrtc.rs`
- `src/srtp.ts` → `rust/zalo-media/src/call/srtp.rs`
- `src/rtcp.ts` → `rust/zalo-media/src/call/rtcp.rs`
- `src/video.ts` → `rust/zalo-media/src/call/video.rs`
- `src/zrtcclient.ts` → `rust/zalo-media/src/call/zrtcclient.rs`
- Call state machine from `src/session.ts` (lines ~1060–1570) + `/api/calls/*` & `/ws/call-audio` from `src/server.ts` (lines ~440–500, 567–613, 706–744) → `rust/zalo-server/src/routes/calls.rs` (and optional `calls_manager.rs` referenced only from `routes/calls.rs`!).
- Existing `zalo-proto` helpers already done: `zalo_proto::api::voicecall::*` (`VoiceCallApi`, `incoming_call_of`, `ringing_call_of`, `media_params_of`, `outgoing_call_of`, `outgoing_media_of`, `our_extend_data`, `new_call_id`) and `zalo_proto::api::ctrlpoll::ControlPoller`.

### Exact Binary Framing for `WS /ws/call-audio` (MUST match `android/.../CallAudioLink.kt`)
- **Server → App:**
  - `0x01 | u16BE(seq) | u32BE(rtp_ts) | opus_payload`
  - `0x03 | u8(flags: bit0=key) | u32BE(90kHz rtp_ts) | H.264 Annex-B AU`
  - `0x05` (1 byte: peer requested key frame)
- **App → Server:**
  - `0x02 | opus_frame` (2..=1276 bytes)
  - `0x04 | u8(flags: bit0=key) | H.264 Annex-B AU` (3..=524290 bytes)
  - `0x06` (app decoder requests key frame → triggers RTCP PLI)
- Close with code `1008` if no active call; replace existing client with `1000`; drop outgoing frames if client channel lags.

### Exact Endpoints to Wire in `rust/zalo-server/src/routes/calls.rs`
- `POST /api/calls/{callId}/decline` → `204`
- `POST /api/calls/{callId}/answer` → `204`
- `POST /api/calls/{callId}/answer-test` → `204` (30s silent test answer)
- `POST /api/calls/start` (`{ userId, video? }`) → `200 CurrentCall`
- `POST /api/calls/hangup` → `204`
- `POST /api/calls/mute` (`{ muted? }`) → `204`
- `POST /api/calls/camera` (`{ on? }`) → `204`
- `GET /api/calls/current` → `200 CurrentCall` or `404 { error }`
- `GET /api/debug/call-log` → `200 text/plain`, `DELETE /api/debug/call-log` → `204` (already working)
- `GET /ws/call-audio` → full bidirectional media relay

### Reference Tests to Port
- `test/zrtc.test.ts`
- `test/srtp.test.ts`
- `test/rtcp.test.ts`
- `test/video.test.ts`
- `test/zrtcclient.test.ts`
- `test/voicecall.test.ts`
- `test/calls.test.ts`
- `test/callaudio.test.ts`
- `test/answer.test.ts`
- `test/ctrlpoll.test.ts`

### Copy-Paste Prompt for Claude Code Account #1
```text
Read RUST_PORT_HANDOFF.md and execute "Track 1: Calls" on a new branch `feat/rust-calls` branched off `feat/rust-backend`.

Strict boundaries (to avoid any conflict with Track 2):
- Only edit/create files under `rust/zalo-media/src/call/` and `rust/zalo-server/src/routes/calls.rs` (plus any helper modules inside `rust/zalo-server/src/routes/` that only `calls.rs` uses). Do NOT touch `rust/zalo-media/src/sync/` or `rust/zalo-server/src/routes/history_sync.rs`.
- `zalo_proto::api::voicecall` and `zalo_proto::api::ctrlpoll` are already implemented in `rust/zalo-proto`.
- Port `src/zrtc.ts`, `src/srtp.ts`, `src/rtcp.ts`, `src/video.ts`, and `src/zrtcclient.ts` faithfully into `rust/zalo-media/src/call/`.
- Wire the full 1:1 incoming/outgoing call state machine (including `ControlPoller` when enabled, WS `voip` events via `session.subscribe_voip_controls()`, `/api/calls/*`, and `/ws/call-audio` binary framing matching `CallAudioLink.kt`).
- Port unit & integration tests from `test/{zrtc,srtp,rtcp,video,zrtcclient,voicecall,calls,callaudio,answer,ctrlpoll}.test.ts`.
- Verify `cargo test --workspace` in `rust/` and `cargo check --target aarch64-linux-android -p zalo-android` pass with 0 errors, then commit and push `feat/rust-calls`.
```

---

## Track 2: History Sync (`feat/rust-history-sync`)

### Reference Files to Port
- `src/lzma.ts` → use `lzma-rs = "0.3"` (pure Rust, zero C deps!) with 256 MiB output cap in `rust/zalo-media/src/sync/lzma.rs`
- `src/zdb.ts` → `rust/zalo-media/src/sync/zdb.rs` (use `rsa = { version = "0.9", features = ["pkcs1", "pkcs8"] }`: RSA-2048 SPKI base64 public key, PKCS#1 v1.5 private decryption, 64 KiB zero-IV AES-256-CBC chunks, `ZDB4.0` archive parser with path-traversal guard)
- `src/sqliteread.ts` → `rust/zalo-media/src/sync/sqliteread.rs` (direct ~250-line pure-Rust port of the read-only SQLite3 binary B-tree/varint/overflow/record parser — **do NOT link `libsqlite3`** to keep APK tiny!)
- `src/syncimport.ts` → `rust/zalo-media/src/sync/syncimport.rs` (`ChatContent` reader, `BinNet` LE/BE TLV parser, `MsgType` map, local Android URI filtering, plain→noised ID translation)
- `src/historysync.ts` → `rust/zalo-server/src/routes/history_sync.rs` (full sync state machine: `idle` → `waiting_for_phone` → `downloading` → `importing` → `done`/`failed`, generation tokens, timeouts, retry/backoff, `pull_mobile_msg` / `cancel_pull_mobile_msg` / `delete_snapshot_mobile_msg` via `zalo_proto::ZaloClient::get_file_service`, manual-redirect backup download via `download_backup`, `/api/znoise` batches of 100 via `get_znoise`, `ChatStore::add_history_message` / `migrate_plain_thread` / `settle_history`, broadcast `{ "type": "history_sync", ... }` + `{ "type": "threads" }`).

### Exact Endpoints to Wire in `rust/zalo-server/src/routes/history_sync.rs`
- `POST /api/history-sync/start` → `202` (or `200` if already active) `HistorySyncStatus`
- `POST /api/history-sync/cancel` → `200 HistorySyncStatus`
- `GET /api/history-sync/status` → `200 HistorySyncStatus & { log: string[] }`

### Reference Tests & Real Binary Fixtures (ALREADY in `test/fixtures/`)
Port tests directly against the real binary fixtures at `../../test/fixtures/`:
- `test/fixtures/lz_text.xz`, `lz_text.lzma`, `lz_big.xz`, `lz_rnd.xz` (from `test/lzma.test.ts`)
- `test/fixtures/sync_1001.db`, `sync_group_777.db`, `sync_many.db` (from `test/sqliteread.test.ts`)
- `test/fixtures/sync_body.xz`, `sync_files.json` (from `test/historysync.test.ts` and `test/syncsender.test.ts`)

### Copy-Paste Prompt for Claude Code Account #2
```text
Read RUST_PORT_HANDOFF.md and execute "Track 2: History Sync" on a new branch `feat/rust-history-sync` branched off `feat/rust-backend`.

Strict boundaries (to avoid any conflict with Track 1):
- Only edit/create files under `rust/zalo-media/src/sync/`, `rust/zalo-media/Cargo.toml` (add `rsa` and `lzma-rs`, pure Rust — do NOT add `rusqlite`/`libsqlite3`!), and `rust/zalo-server/src/routes/history_sync.rs` (plus any helper modules inside `rust/zalo-server/src/routes/` that only `history_sync.rs` uses). Do NOT touch `rust/zalo-media/src/call/` or `rust/zalo-server/src/routes/calls.rs`.
- `ZaloClient::get_file_service`, `get_znoise`, and `download_backup` are already implemented in `rust/zalo-proto`.
- Port `src/zdb.ts`, `src/sqliteread.ts`, `src/syncimport.ts` into `rust/zalo-media/src/sync/` (using `lzma-rs` + `rsa`), and `src/historysync.ts` into `rust/zalo-server/src/routes/history_sync.rs`.
- Wire `session.subscribe_history_sync_controls()`, `POST /api/history-sync/start`, `POST /api/history-sync/cancel`, and `GET /api/history-sync/status`.
- Port tests from `test/{lzma,sqliteread,historysync,syncsender}.test.ts` verifying against the real binary fixtures in `test/fixtures/`.
- Verify `cargo test --workspace` in `rust/` and `cargo check --target aarch64-linux-android -p zalo-android` pass with 0 errors, then commit and push `feat/rust-history-sync`.
```

---

## Final Merge & Cleanup (After Both Tracks Pass)
```bash
git checkout feat/rust-backend
git merge --no-ff feat/rust-calls
git merge --no-ff feat/rust-history-sync
cd rust && cargo test --workspace
cd ../android && ./gradlew --no-daemon testDebugUnitTest assembleDebug assembleRelease
```
Optional final cleanup once verified on a real phone: remove legacy `src/`, `test/` (keeping any fixtures moved to `rust/zalo-media/tests/fixtures/`), `package.json`, `package-lock.json`, `tsconfig.json`, and `RUST_PORT_HANDOFF.md`.
