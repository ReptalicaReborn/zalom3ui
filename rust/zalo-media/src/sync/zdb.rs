//! ZDB4.0 phone history-sync backup cryptography and archive unpacking (`src/zdb.ts` port).
//!
//! Format:
//! 1. Ephemeral RSA-2048 key pair; SPKI DER base64 public key is sent to the phone.
//! 2. Phone replies with an RSAES-PKCS1-v1_5 encrypted backup key (base64).
//! 3. Backup key as text (hex for `db_format` 1, base64 for 0) -> first 32 ASCII bytes uppercased -> AES-256 key.
//! 4. Archive is AES-256-CBC, zero IV reset every 64 KiB, no padding, optionally framed with 4 leading/trailing bytes.
//! 5. Plaintext starts with `"ZDB4.0"` (18-byte header, bytes 14..18 = BE u32 file count),
//!    followed by `(u32 BE name_len, utf8 name, u32 BE file_size)` entries and one LZMA/XZ stream.

use aes::Aes256;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use rand::rngs::OsRng;
use rsa::pkcs8::EncodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};

use super::lzma::decode_lzma_auto;
use super::sqliteread::is_sqlite;

type Aes256CbcDec = cbc::Decryptor<Aes256>;
type Aes256CbcEnc = cbc::Encryptor<Aes256>;

pub const MAGIC: &[u8; 6] = b"ZDB4.0";
pub const MAX_FILES: usize = 100_000;
const CHUNK_SIZE: usize = 0x10000;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct BackupError(pub String);

impl BackupError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Ephemeral RSA-2048 key pair for a single history-sync session.
#[derive(Clone)]
pub struct SyncKeyPair {
    pub public_key_spki_b64: String,
    pub private_key: RsaPrivateKey,
}

impl std::fmt::Debug for SyncKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncKeyPair")
            .field("public_key_spki_b64_len", &self.public_key_spki_b64.len())
            .finish()
    }
}

/// Generates a fresh 2048-bit RSA key pair with SPKI DER base64 public key.
pub fn generate_sync_keypair() -> Result<SyncKeyPair, BackupError> {
    let mut rng = OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| BackupError::new(format!("RSA key generation failed: {e}")))?;
    let der = private_key
        .to_public_key()
        .to_public_key_der()
        .map_err(|e| BackupError::new(format!("SPKI encoding failed: {e}")))?;
    Ok(SyncKeyPair {
        public_key_spki_b64: B64.encode(der.as_bytes()),
        private_key,
    })
}

/// Decrypts the phone's RSAES-PKCS1-v1_5 encrypted backup key (base64).
/// Returns `None` on any failure without leaking padding-oracle details.
pub fn decrypt_backup_key(priv_key: &RsaPrivateKey, encrypted_b64: &str) -> Option<Vec<u8>> {
    let trimmed = encrypted_b64.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cipher = B64.decode(trimmed).ok()?;
    if cipher.is_empty() || cipher.len() > priv_key.size() {
        return None;
    }
    let plain = priv_key.decrypt(Pkcs1v15Encrypt, &cipher).ok()?;
    if plain.is_empty() {
        None
    } else {
        Some(plain)
    }
}

/// Encrypts a plaintext backup key with RSAES-PKCS1-v1_5 to `key_pair`'s public key (base64),
/// mirroring what the phone does (used by unit/integration tests).
pub fn encrypt_backup_key_pkcs1(
    key_pair: &SyncKeyPair,
    plain_key: &[u8],
) -> Result<String, BackupError> {
    let pub_key = key_pair.private_key.to_public_key();
    let mut rng = OsRng;
    let cipher = pub_key
        .encrypt(&mut rng, Pkcs1v15Encrypt, plain_key)
        .map_err(|e| BackupError::new(format!("RSA encryption failed: {e}")))?;
    Ok(B64.encode(cipher))
}

/// Formats the decrypted backup key as text: lowercase hex for `db_format != 0`, base64 for `0`.
pub fn backup_key_text(plain_key: &[u8], format: u32) -> String {
    if format != 0 {
        hex::encode(plain_key)
    } else {
        B64.encode(plain_key)
    }
}

/// Derives the 32-byte AES-256 key from the backup key text: first 32 ASCII/latin1 bytes,
/// ASCII lowercase uppercased, zero-padded if shorter.
pub fn derive_backup_aes_key(key_text: &str) -> [u8; 32] {
    let mut key = [0u8; 32];
    for (dst, ch) in key.iter_mut().zip(key_text.chars()) {
        let c = (ch as u32 & 0xff) as u8;
        *dst = if (0x61..=0x7a).contains(&c) {
            c - 0x20
        } else {
            c
        };
    }
    key
}

/// Raw AES-256-CBC decryption with zero IV restarted every 64 KiB and no padding.
pub fn decrypt_chunks_raw(data: &[u8], aes_key: &[u8; 32]) -> Result<Vec<u8>, BackupError> {
    if data.len() % 16 != 0 {
        return Err(BackupError::new(
            "The backup is not a whole number of AES blocks",
        ));
    }
    let mut out = data.to_vec();
    let zero_iv = [0u8; 16];
    for chunk in out.chunks_mut(CHUNK_SIZE) {
        let dec = Aes256CbcDec::new(aes_key.into(), (&zero_iv).into());
        dec.decrypt_padded_mut::<NoPadding>(chunk)
            .map_err(|_| BackupError::new("AES-256-CBC chunk decryption failed"))?;
    }
    Ok(out)
}

/// Inverse of [`decrypt_chunks_raw`] (AES-256-CBC, zero IV reset every 64 KiB, no padding).
pub fn encrypt_chunks_raw(data: &[u8], aes_key: &[u8; 32]) -> Result<Vec<u8>, BackupError> {
    if data.len() % 16 != 0 {
        return Err(BackupError::new(
            "Plaintext is not a whole number of AES blocks",
        ));
    }
    let mut out = data.to_vec();
    let zero_iv = [0u8; 16];
    for chunk in out.chunks_mut(CHUNK_SIZE) {
        let enc = Aes256CbcEnc::new(aes_key.into(), (&zero_iv).into());
        enc.encrypt_padded_mut::<NoPadding>(chunk, chunk.len())
            .map_err(|_| BackupError::new("AES-256-CBC chunk encryption failed"))?;
    }
    Ok(out)
}

/// Probes optional ±4-byte framing for the `"ZDB4.0"` magic and decrypts the archive
/// using AES-256-CBC with zero IV reset every 64 KiB.
pub fn decrypt_zdb_chunks(ciphertext: &[u8], aes_key: &[u8; 32]) -> Result<Vec<u8>, BackupError> {
    let size = ciphertext.len();
    if size < 32 {
        return Err(BackupError::new("The backup file is too small"));
    }

    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(3);
    if size % 16 == 0 {
        spans.push((0, size));
    }
    if size > 4 && (size - 4) % 16 == 0 {
        spans.push((4, size - 4));
        spans.push((0, size - 4));
    }
    if spans.is_empty() {
        return Err(BackupError::new(
            "The backup file is not a whole number of AES blocks",
        ));
    }

    let zero_iv = [0u8; 16];
    let mut matched_span = None;
    for &(off, len) in &spans {
        let probe_len = len.min(64);
        let mut probe = ciphertext[off..off + probe_len].to_vec();
        let dec = Aes256CbcDec::new(aes_key.into(), (&zero_iv).into());
        if dec.decrypt_padded_mut::<NoPadding>(&mut probe).is_ok()
            && probe.len() >= 18
            && &probe[..6] == MAGIC
        {
            matched_span = Some((off, len));
            break;
        }
    }

    let Some((off, len)) = matched_span else {
        return Err(BackupError::new("Wrong key, or not a ZDB4 backup"));
    };

    decrypt_chunks_raw(&ciphertext[off..off + len], aes_key)
}

/// A single extracted file entry from a ZDB4.0 archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZdbEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Unpacks an already-decrypted ZDB4.0 archive buffer into its file entries.
pub fn unpack_zdb(decrypted: &[u8], max_output: usize) -> Result<Vec<ZdbEntry>, BackupError> {
    if decrypted.len() < 18 || &decrypted[..6] != MAGIC {
        return Err(BackupError::new("Wrong key, or not a ZDB4 backup"));
    }
    let count = u32::from_be_bytes(decrypted[14..18].try_into().unwrap()) as usize;
    if count > MAX_FILES {
        return Err(BackupError::new("The backup lists too many files"));
    }

    let mut table: Vec<(String, usize)> = Vec::with_capacity(count.min(1024));
    let mut pos = 18usize;
    let mut total = 0usize;

    for _ in 0..count {
        if pos + 4 > decrypted.len() {
            return Err(BackupError::new("The backup's file table is cut short"));
        }
        let name_len = u32::from_be_bytes(decrypted[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if name_len > 4096 || pos.saturating_add(name_len).saturating_add(4) > decrypted.len() {
            return Err(BackupError::new("The backup's file table is not valid"));
        }
        let name = std::str::from_utf8(&decrypted[pos..pos + name_len])
            .map_err(|_| BackupError::new("The backup's file table has invalid UTF-8"))?
            .to_string();
        pos += name_len;
        let file_size = u32::from_be_bytes(decrypted[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        if name.is_empty()
            || name.contains("..")
            || name.starts_with('/')
            || name.contains('\\')
            || name.contains('\0')
        {
            return Err(BackupError::new("The backup has an unsafe file name"));
        }
        total = total
            .checked_add(file_size)
            .ok_or_else(|| BackupError::new("The backup is larger than the allowed maximum"))?;
        if total > max_output {
            return Err(BackupError::new(
                "The backup is larger than the allowed maximum",
            ));
        }
        table.push((name, file_size));
    }

    if table.is_empty() {
        return Ok(Vec::new());
    }

    let body = decode_lzma_auto(&decrypted[pos..], max_output)
        .map_err(|e| BackupError::new(format!("The backup's data is damaged ({e})")))?;
    if body.len() < total {
        return Err(BackupError::new(
            "The backup ended before all its files were read",
        ));
    }

    let mut files = Vec::with_capacity(table.len());
    let mut at = 0usize;
    for (name, file_size) in table {
        files.push(ZdbEntry {
            name,
            data: body[at..at + file_size].to_vec(),
        });
        at += file_size;
    }
    Ok(files)
}

/// Full decrypt + unpack pipeline for an encrypted ZDB4.0 backup.
pub fn unpack_backup(
    encrypted: &[u8],
    key_text: &str,
    max_output: usize,
) -> Result<Vec<ZdbEntry>, BackupError> {
    let key = derive_backup_aes_key(key_text);
    let plain = decrypt_zdb_chunks(encrypted, &key)?;
    unpack_zdb(&plain, max_output)
}

/// Checks whether an archive entry filename matches a conversation SQLite database:
/// `<id>.db`, `group_<id>.db`, or `g<id>.db` (returns `(id, is_group)`).
pub fn is_conversation_db(name: &str) -> Option<(String, bool)> {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name);
    let stem = base.strip_suffix(".db")?;
    let (id, is_group) = if let Some(rest) = stem.strip_prefix("group_") {
        (rest, true)
    } else if let Some(rest) = stem.strip_prefix('g') {
        if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
            (rest, true)
        } else {
            return None;
        }
    } else {
        (stem, false)
    };
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        Some((id.to_string(), is_group))
    } else {
        None
    }
}

/// A validated conversation SQLite database extracted from a backup.
#[derive(Debug, Clone)]
pub struct ConversationFile {
    pub id: String,
    pub is_group: bool,
    pub data: Vec<u8>,
}

/// Filters archive entries to valid SQLite conversation DBs, sorted by `(id, is_group)`.
pub fn conversation_files(entries: &[ZdbEntry]) -> Vec<ConversationFile> {
    let mut out = Vec::new();
    for entry in entries {
        if !is_sqlite(&entry.data) {
            continue;
        }
        if let Some((id, is_group)) = is_conversation_db(&entry.name) {
            out.push(ConversationFile {
                id,
                is_group,
                data: entry.data.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.is_group.cmp(&b.is_group)));
    out
}

/// Packs a ZDB4.0 archive with an already-compressed LZMA/XZ body (used by integration/unit tests).
pub fn pack_zdb_backup(
    files: &[(&str, usize)],
    compressed_body: &[u8],
    key_text: &str,
    prefix_bytes: usize,
    suffix_bytes: usize,
) -> Result<Vec<u8>, BackupError> {
    let mut plain = Vec::new();
    let mut head = [0u8; 18];
    head[..6].copy_from_slice(MAGIC);
    head[14..18].copy_from_slice(&(files.len() as u32).to_be_bytes());
    plain.extend_from_slice(&head);
    for &(name, size) in files {
        let name_bytes = name.as_bytes();
        plain.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
        plain.extend_from_slice(name_bytes);
        plain.extend_from_slice(&(size as u32).to_be_bytes());
    }
    plain.extend_from_slice(compressed_body);
    let rem = plain.len() % 16;
    if rem != 0 {
        plain.resize(plain.len() + (16 - rem), 0);
    }
    let key = derive_backup_aes_key(key_text);
    let enc = encrypt_chunks_raw(&plain, &key)?;
    let mut out = Vec::with_capacity(prefix_bytes + enc.len() + suffix_bytes);
    out.resize(prefix_bytes, 0x5a);
    out.extend_from_slice(&enc);
    out.resize(out.len() + suffix_bytes, 0x5a);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::lzma::DEFAULT_MAX_OUTPUT;
    use rsa::Oaep;
    use sha1::Sha1;
    use std::path::PathBuf;

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
    }

    const KEY_TEXT: &str = "00112233445566778899aabbccddeeff";

    #[test]
    fn derive_aes_key_and_key_text() {
        let key = derive_backup_aes_key("0123456789abcdefABCDEF0123456789zzzz");
        assert_eq!(&key, b"0123456789ABCDEFABCDEF0123456789");

        let short = derive_backup_aes_key("abc");
        assert_eq!(&short[..3], b"ABC");
        assert!(short[3..].iter().all(|&b| b == 0));

        assert_eq!(backup_key_text(&[0xde, 0xad, 0xbe, 0xef], 1), "deadbeef");
        assert_eq!(backup_key_text(&[0xde, 0xad, 0xbe, 0xef], 0), "3q2+7w==");
    }

    #[test]
    fn aes_chunks_restart_zero_iv_every_64kib() {
        let key = derive_backup_aes_key(&"K".repeat(32));
        let plain = vec![7u8; 0x10000 + 64];
        let enc = encrypt_chunks_raw(&plain, &key).unwrap();
        assert_eq!(&enc[..16], &enc[0x10000..0x10000 + 16]);
        assert_eq!(decrypt_chunks_raw(&enc, &key).unwrap(), plain);
        assert!(decrypt_chunks_raw(&enc[..30], &key).is_err());
    }

    #[test]
    fn rsa_pkcs1v15_roundtrip_and_rejects_oaep_and_garbage() {
        let pair = generate_sync_keypair().unwrap();
        let pub_key = pair.private_key.to_public_key();
        let mut rng = OsRng;
        let secret = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00];
        let cipher = pub_key.encrypt(&mut rng, Pkcs1v15Encrypt, &secret).unwrap();
        let back = decrypt_backup_key(&pair.private_key, &B64.encode(&cipher)).unwrap();
        assert_eq!(back, secret);

        // Garbage & empty
        assert!(decrypt_backup_key(&pair.private_key, &B64.encode(vec![0xaa; 256])).is_none());
        assert!(decrypt_backup_key(&pair.private_key, "").is_none());

        // OAEP is rejected
        let oaep = pub_key.encrypt(&mut rng, Oaep::new::<Sha1>(), &secret).unwrap();
        assert!(decrypt_backup_key(&pair.private_key, &B64.encode(&oaep)).is_none());
    }

    #[test]
    fn unpack_backup_with_real_fixtures_and_framing() {
        let files_json: Vec<(String, usize)> =
            serde_json::from_slice(&fixture("sync_files.json")).unwrap();
        let specs: Vec<(&str, usize)> = files_json.iter().map(|(n, s)| (n.as_str(), *s)).collect();
        let body = fixture("sync_body.xz");

        let packed = pack_zdb_backup(&specs, &body, KEY_TEXT, 0, 0).unwrap();
        let entries = unpack_backup(&packed, KEY_TEXT, DEFAULT_MAX_OUTPUT).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, files_json[0].0);
        assert_eq!(entries[0].data, fixture("sync_1001.db"));
        assert_eq!(entries[1].name, files_json[1].0);
        assert_eq!(entries[1].data, fixture("sync_group_777.db"));

        let convs = conversation_files(&entries);
        let ids: Vec<(&str, bool)> = convs.iter().map(|c| (c.id.as_str(), c.is_group)).collect();
        assert_eq!(ids, vec![("1001", false), ("777", true)]);

        // ±4-byte framing tolerated
        for (pre, suf) in [(4, 0), (0, 4)] {
            let framed = pack_zdb_backup(&specs, &body, KEY_TEXT, pre, suf).unwrap();
            let unpacked = unpack_backup(&framed, KEY_TEXT, DEFAULT_MAX_OUTPUT).unwrap();
            assert_eq!(unpacked.len(), 2);
        }
    }

    #[test]
    fn unpack_backup_rejects_wrong_key_bad_names_and_corrupt_body() {
        let files_json: Vec<(String, usize)> =
            serde_json::from_slice(&fixture("sync_files.json")).unwrap();
        let specs: Vec<(&str, usize)> = files_json.iter().map(|(n, s)| (n.as_str(), *s)).collect();
        let body = fixture("sync_body.xz");
        let packed = pack_zdb_backup(&specs, &body, KEY_TEXT, 0, 0).unwrap();

        let err = unpack_backup(&packed, "ffeeddccbbaa99887766554433221100", DEFAULT_MAX_OUTPUT)
            .unwrap_err();
        assert!(err.to_string().contains("Wrong key"));

        assert!(unpack_backup(&packed[..40], KEY_TEXT, DEFAULT_MAX_OUTPUT).is_err());
        assert!(unpack_backup(&vec![1u8; 100], KEY_TEXT, DEFAULT_MAX_OUTPUT).is_err());

        let evil = pack_zdb_backup(&[("../evil.db", 1)], &body, KEY_TEXT, 0, 0).unwrap();
        let err = unpack_backup(&evil, KEY_TEXT, DEFAULT_MAX_OUTPUT).unwrap_err();
        assert!(err.to_string().contains("unsafe"));

        let oversized = pack_zdb_backup(&[("1001.db", 10_000_000)], &body, KEY_TEXT, 0, 0).unwrap();
        assert!(unpack_backup(&oversized, KEY_TEXT, DEFAULT_MAX_OUTPUT).is_err());

        let bad_body = pack_zdb_backup(&specs, b"garbage not lzma", KEY_TEXT, 0, 0).unwrap();
        assert!(unpack_backup(&bad_body, KEY_TEXT, DEFAULT_MAX_OUTPUT).is_err());
    }

    #[test]
    fn conversation_files_three_naming_schemes_and_sqlite_only() {
        let db = fixture("sync_1001.db");
        let found = conversation_files(&[
            ZdbEntry { name: "100.db".into(), data: db.clone() },
            ZdbEntry { name: "group_200.db".into(), data: db.clone() },
            ZdbEntry { name: "g300.db".into(), data: db.clone() },
            ZdbEntry { name: "db/400.db".into(), data: db.clone() },
            ZdbEntry { name: "abc.db".into(), data: db.clone() },
            ZdbEntry { name: "500.db".into(), data: b"not sqlite at all, but long enough to look like something".to_vec() },
            ZdbEntry { name: "readme.txt".into(), data: db },
        ]);
        let labels: Vec<String> = found
            .iter()
            .map(|c| format!("{}{}", if c.is_group { "g" } else { "u" }, c.id))
            .collect();
        assert_eq!(labels, vec!["u100", "g200", "g300", "u400"]);
    }
}
