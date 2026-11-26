# Sync messages from the phone

A fresh login only has the history Zalo's servers replay (recent messages, and for groups a bit more). Zalo PC can ask
the official Zalo app on the phone to hand over its chat history: the phone shows a prompt, then uploads an encrypted
backup, which the PC downloads, decrypts and imports. This document is the protocol as far as we know it, what is
confirmed and what is a guess, and what a real test needs. Code: `src/historysync.ts` (the flow), `src/zdb.ts`
(crypto and container), `src/lzma.ts`, `src/sqliteread.ts`, `src/syncimport.ts` (the backup's contents), `ui/HistorySync.kt`
(the app).

**Sources.** *Confirmed from code* means written in the Zalos project (`src/core/sync/mobile_history_sync.cpp`,
`src/core/crypto/db_cross.cpp`, `src/core/network/zalo_session.cpp`, MIT; we mirror it, our TypeScript is our own) and,
where marked **[PC]**, also in Zalo PC's own web bundle. *Guessed* means neither source says it, or it differs in our
setting (Zalo Web instead of Zalo PC). Nothing here has been run against a real phone yet.

## 1. Overview

```
Zalỏ server                         Zalo servers                    phone (official app)
   | generate RSA-2048 pair             |                                  |
   |-- GET pull_mobile_msg (pub key) -->|-- "syncmsgmb" push ------------->| prompt: allow?
   |                                    |<-- user_confirm ----------------|
   |<-- control: user_confirm ----------|                                  |
   |                                    |<-- upload encrypted backup ------|
   |<-- control: syncmsg_info ----------| (url, encrypted_key, db_info)    |
   |-- GET url (cookies) -------------->|                                  |
   |-- RSA-decrypt key, AES, LZMA, SQLite, import                          |
   |-- GET delete_snapshot_mobile_msg ->|                                  |
```

## 2. Requests (confirmed from code, **[PC]** for the paths)

All three are `GET`s to the **file service** (`zpw_service_map.file[0]`, Zalo PC calls it `getFileDomain()`), at
`<file host>/api/message/<endpoint>?zpw_ver=…&zpw_type=…&params=<AES(JSON)>`, with the usual zca-js parameter
encryption (the same as every other zca-js call; `src/voicecall.ts` does it for calls). The answer is the usual
`{error_code, data}`.

| Endpoint | Parameters | Zalo PC command id |
|---|---|---|
| `pull_mobile_msg` | `pc_name`, `public_key`, `from_seq_id`, `is_retry`, `min_seq_id`, `temp_key`, `imei` | 12000 |
| `cancel_pull_mobile_msg` | `pc_name`, `public_key`, `imei` | 12700 |
| `delete_snapshot_mobile_msg` | `public_key`, `imei` | 12003 |

- `pc_name`: the device name the phone's prompt shows; it comes back in the phone's events, which must match. We send
  `Zalo-Android` (Zalo PC sends the host name). Guess: the phone may only accept names of PCs; unproven for a web login.
- `public_key`: our one-off RSA-2048 public key, SPKI DER, base64 (`generateKeyPairSync("rsa", {type: "spki", format: "der"})`
  in Zalo PC). It also identifies the sync: events carry it back as `public_key`.
- `from_seq_id`: `0` for "everything" (Zalos sends `seq + 1` when resuming at `seq`, else `0`); `is_retry` 0/1;
  `min_seq_id` 0. We always send 0, 0, 0.
- `temp_key`: from the `transfer_after_login` control the phone sends right after a QR login (`data.temp_key` or
  `data.tempKey`, valid for `data.imei` = this device). Empty when none arrived. Zalo PC checks that
  `imei === getZaloClientID()` **[PC]**; we keep the last one we saw.
- Error code **211** ("client not supported") means the phone wants the newer *sync v2* (section 8); Zalos falls back to
  it. We report it as unsupported.
- Other endpoints Zalo PC knows and we don't use: `get_backupmsginfo` (12096), `get_crossdb` (12412, takes
  `pc_name` and `sync_session`; probably the old path), `/api/transfer-sync-v2/request-sync` (v2).

## 3. Events from the phone (confirmed from code, **[PC]** for the predicates)

They arrive as **controls** (`act_type` / `act` / `data`) like friend and group events: on the chat socket as `cmd 601`
(zca-js decodes it but drops these), and, for accounts that use it, from the control-channel poll (`src/ctrlpoll.ts`).
We read both (`session.ts` hooks, `historysync.ts` `syncControlOf`). `content.data` may be a JSON string.

`act_type` `syncmsgmb`, matched to our sync by `data.public_key === ours` and `data.pc_name === ours`:

| `act` | Fields | Meaning |
|---|---|---|
| `user_confirm` | `user_action` | 1 or 3: allowed. 0: declined. 2: the phone is restoring data, try later. |
| `transfer_error` | `status`, `error_code` | `status` 2: phone idle (keep the app open, screen on). 1: phone is working. Else an `error_code` ≠ 0 is a failure. Zalos has a table of phone error names; we print the number only. |
| `syncmsg_info` | `url`, `file_name`, `file_size`, `checksum_code` (or `checksum`), `encrypted_key`, `db_info`, `uid` | The backup is ready. |

Other `act_type`s seen around it: `transfer_after_login` (the `temp_key`, above), `transfer_msg`, and `transfer_sync2`
(sync v2 traffic; Zalos treats a confirm timeout after one as the cue to switch to v2).

`db_info` is a JSON **string**: `{"db_format": 1, "origin_db": {"msg_total": N, …}}` (or `backup_db` instead of
`origin_db`). `db_format` 1 is the format below; 0 is an older one Zalos doesn't read either, and neither do we.
`msg_total` 0 means the phone has nothing to send.

Timeouts (Zalos): 3 minutes for the phone to confirm, 30 minutes for the backup, 2 minutes of download silence.

## 4. Download (confirmed from code)

`GET url` with the session's Zalo cookies (Zalos sends them to Zalo domains only). The file is prepared asynchronously,
so the first tries may fail: Zalos retries HTTP 400, 404, 408, 429, 5xx and network errors up to 5 attempts, 1, 2, 4,
8 s apart, honouring `Retry-After` (seconds or date) for 429/503, at most 30 s. We do the same
(`downloadRetryDelayMs`). It then compares the byte count with `Content-Length` and logs an MD5 against
`checksum_code` without failing on a mismatch. We keep the whole file in memory (cap 256 MB) and don't check the MD5
(guess: the AES layer and the SQLite header checks catch a damaged file).

## 5. Decryption (confirmed from code, **[PC]** for the RSA part)

1. **Key.** `encrypted_key` is base64 of RSAES-PKCS1-v1_5 (`RSA_PKCS1_PADDING`) under our public key. Zalo PC:
   `privateDecrypt(key, RSA_PKCS1_PADDING, encrypted_key, {input: "base64", output: format ? "hex" : "base64"})`
   **[PC]**: the plaintext key is turned into **text**, hex for `db_format` 1, base64 for 0. Node 20.11+ refuses PKCS#1
   v1.5 private-key decryption, so `rsaDecryptPkcs1` does a raw RSA operation and removes the padding itself.
2. **AES key** = the first 32 characters of that text, ASCII `a-z` made `A-Z`, as bytes, zero-padded
   (`DbCross::deriveAes256Key`). For a 16-byte hex key that is the 32 upper-case hex characters.
3. **Cipher**: AES-256-CBC, IV all zero, **restarted every 0x10000 bytes** (each 64 KiB chunk decrypts independently),
   no padding. The file size must be a multiple of 16, or of 16 plus 4 (four extra bytes at the start or at the end;
   Zalos tries `[0..n]`, `[4..n]`, `[0..n-4]` and takes the layout whose first 6 decrypted bytes are `ZDB4.0`).
4. **Container** (decrypted): 18 header bytes starting `ZDB4.0`, with the file count as big-endian u32 at offset 14
   (the other bytes are unknown to us); then per file `u32 name length`, name, `u32 size` (big-endian); then **one LZMA
   stream** that decompresses to all files back to back, in table order. Names with `..`, a leading `/` or a backslash
   are refused.
5. **LZMA**: liblzma's `lzma_auto_decoder` is used, so either `.xz` (LZMA2) or the old `.lzma` stream. Guess: which one
   the phone really sends isn't written anywhere; `src/lzma.ts` reads both (xz with a single LZMA2 filter only, checksums
   skipped).

## 6. Contents (confirmed from code)

Files `<id>.db`, `group_<id>.db` or `g<id>.db` (id = digits): one SQLite database per conversation, with a table
`ChatContent` (rows for messages). Used columns (first match wins): `GlbMsgId`/`GlobalMsgId`/`MsgId`, `CliMsgId`,
`MsgType`, `TimeStamp`/`Ts`, `SenderId`/`FromId`/`FromUid`, `MsgContent`/`Message`, `TTL`, `BinNet` (blob), `MsgStatus`.
Zalos reads rows `WHERE MsgStatus > 0 AND CliMsgId NOT NULL ORDER BY TimeStamp`; so do we (in JS: `src/sqliteread.ts`
is a small read-only SQLite file reader, because the phone's embedded Node 18 has no SQLite).

- **Sender**: `"0"` means sent from this account (as in live messages), else the plain user id.
- **Ids**: The phone backup uses plain 8–9 digit ids, whereas Zalo Web (`zca-js`) and Zalo PC use 18–19 digit
  "noised" ids everywhere. Before importing, we map plain user/group ids (`fids`/`gids`) via `GET /api/znoise` on the
  profile service (in batches of 100), and use `syncmsg_info.data.uid` as our own plain id.
- **`BinNet`**: a flat TLV box, `key u32, length u32, value`, byte order unknown per blob (Zalos parses both and keeps
  the one that scores more known tags). Tags: `0x0b` text, `0x0a` data source, `0x09` quote status, `0x06` attachment
  (repeats; inside: `0x28` type, `0x29` catId, `0x2a` id, `0x2b` extInfo, `0x2d` action, `0x2e` params, `0x2f` title,
  `0x30` href, `0x31` thumb, `0x32` description, `0x33` size, `0x34` ext), `0x07` quote (`0x50` owner, `0x51` cliMsgId,
  `0x52` globalMsgId, `0x53` type, `0x54` ts, `0x56` text, `0x57` attach, `0x5a` from name), `0x08` mention
  (repeats; `0x65` uid, `0x66` pos, `0x67` len, `0x68` type). Text may be UTF-8 or (ASCII-heavy) UTF-16LE.
- **`MsgType`** to message type: 2 doodle, 3/4/31 photo, 6 voice, 10 sticker, 12 recommended (link/contact), 18
  location, 19 video, 22 file, 23 gif, 24/53 web content, 26 poll, 36 recall (the row's text is JSON with
  `globalMsgId`), anything else text. Types 21, 25, 29, 32-35, 45, 51, 52 are not chat content and are skipped.
- **Timestamps**: milliseconds (guess; a value below 1e11 is treated as seconds).

## 7. Import (ours)

`importConversation` converts rows to zca-js's raw message shape and runs `normalizeMessage`, so imported messages look
like live ones. New ids only (`ChatStore` dedupes by id; a message already stored wins), in timestamp order. Recall rows
mark their target recalled. Imported messages never count as unread and trigger no notification (`settleHistory`);
they have no `raw` copy, so they can't be quoted or reacted to ("too old to act on"). History raises the per-chat
limit from 500 to 2000 messages (`ChatStore.allowMessages`, saved in the cache); older messages in a bigger chat are
not kept. Guess to check: the id the phone stores as `GlbMsgId` equals the `msgId` live messages carry; if it doesn't,
a re-sync would duplicate messages.

## 8. Sync v2 (not implemented)

Zalo on iPhone (and, Zalos says, some newer clients) answers `pull_mobile_msg` with code 211. Zalos then speaks a
different protocol: a `590` WebSocket request (`syncId`, `ek`/`ik` Curve25519 keys, `req` with queries per partition),
`591` release, `592` wake-up, replies as `transfer_sync2` controls pointing at encrypted batches (an X3DH-style
session, protobuf frames, in `src/core/sync/v2/`). That is a second project of its own; we report "not supported" and
log the event names.

## 9. API (ours)

- `POST /api/history-sync/start`: asks the phone; 202 with the status (200 if one is already running; 409 when logged out).
- `POST /api/history-sync/cancel`: stops, tells Zalo (`cancel_pull_mobile_msg`), back to `idle`.
- `GET /api/history-sync/status`: the status plus `log`, the last 200 lines (event names, field names, counts, sizes;
  never message text, keys or the download link).
- `/ws` events `{type: "history_sync", state, progress, error, detail, expectedMessages?, counts?}`; `state` is `idle`,
  `waiting_for_phone`, `downloading`, `importing`, `done` or `failed`. When it is `done` a `{type: "threads"}` follows,
  so clients reload their chat list.

## 10. What a real test needs

Needs a phone with the official Zalo (Android first; sync v1) logged in to the same account, awake and unlocked, and
the server logged in by QR. Then `POST /api/history-sync/start` (or Settings, "Sync messages from phone"). Things that
may fail, and what to look at in `GET /api/history-sync/status` → `log`:

1. **The request is refused** (`pull_mobile_msg failed code=N`): a web login may not be allowed, or `pc_name` is
   refused. 211 is sync v2.
2. **The phone shows no prompt / no controls arrive.** The log has a `control …` line for every sync-related control
   with its field names. If none appears the controls come over another channel; then log all `act_type`s (the call
   log, `GET /api/debug/call-log`, already lists them as `controls` entries).
3. **`syncmsg_info` arrives but with other field names**: the `backup info:` line lists the keys of `data` and `db_info`.
4. **The download fails**: HTTP status in the log; maybe cookies or a header are needed beyond `utils.request`'s.
5. **"Wrong key"**: the key text rule (hex vs base64, upper-casing) or the 4-byte layout; the container header is the
   first thing to dump (offsets 6-13 are unknown).
6. **"damaged data" from the LZMA step**: probably a different container than `.xz`/`.lzma`.
7. **Messages in the wrong chats or twice**: the id mapping (section 6, noised ids) and the `GlbMsgId` question
   (section 7).
