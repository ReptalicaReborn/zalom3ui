use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Read;

use aes::{Aes128, Aes192, Aes256};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{AesGcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use md5::{Digest, Md5};
use rand::Rng;
use serde_json::Value;
use typenum::U16;
use uuid::Uuid;

use crate::error::{ProtoError, Result};

type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes192CbcEnc = cbc::Encryptor<Aes192>;
type Aes256CbcEnc = cbc::Encryptor<Aes256>;

type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes192CbcDec = cbc::Decryptor<Aes192>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

type Aes128Gcm16 = AesGcm<Aes128, U16>;
type Aes192Gcm16 = AesGcm<Aes192, U16>;
type Aes256Gcm16 = AesGcm<Aes256, U16>;

/// Cached AES-GCM cipher built once per WebSocket connection from the handshake `cipherKey`.
pub enum EventCipher {
    Aes128Gcm16(Aes128Gcm16),
    Aes192Gcm16(Aes192Gcm16),
    Aes256Gcm16(Aes256Gcm16),
}

impl EventCipher {
    /// Expands a base64-encoded 16-, 24-, or 32-byte AES-GCM key once for reuse across frames.
    pub fn from_base64_key(cipher_key_b64: &str) -> Result<Self> {
        let key = B64
            .decode(cipher_key_b64.trim())
            .map_err(|e| ProtoError::Crypto(format!("invalid base64 cipher_key: {e}")))?;
        match key.len() {
            16 => Ok(Self::Aes128Gcm16(
                Aes128Gcm16::new_from_slice(&key)
                    .map_err(|e| ProtoError::Crypto(e.to_string()))?,
            )),
            24 => Ok(Self::Aes192Gcm16(
                Aes192Gcm16::new_from_slice(&key)
                    .map_err(|e| ProtoError::Crypto(e.to_string()))?,
            )),
            32 => Ok(Self::Aes256Gcm16(
                Aes256Gcm16::new_from_slice(&key)
                    .map_err(|e| ProtoError::Crypto(e.to_string()))?,
            )),
            n => Err(ProtoError::Crypto(format!(
                "unsupported AES-GCM key length: {n} bytes"
            ))),
        }
    }

    pub fn decrypt(&self, nonce: &[u8; 16], aad: &[u8], msg: &[u8]) -> Result<Vec<u8>> {
        let nonce = Nonce::<U16>::from_slice(nonce);
        let payload = Payload { msg, aad };
        match self {
            Self::Aes128Gcm16(c) => c
                .decrypt(nonce, payload)
                .map_err(|e| ProtoError::Crypto(format!("AES-128-GCM decrypt failed: {e}"))),
            Self::Aes192Gcm16(c) => c
                .decrypt(nonce, payload)
                .map_err(|e| ProtoError::Crypto(format!("AES-192-GCM decrypt failed: {e}"))),
            Self::Aes256Gcm16(c) => c
                .decrypt(nonce, payload)
                .map_err(|e| ProtoError::Crypto(format!("AES-256-GCM decrypt failed: {e}"))),
        }
    }
}

const ZCID_KEY: &str = "3FC4F0D2AB50057BCE0D90D9187A22B1";
const ZERO_IV: [u8; 16] = [0u8; 16];
pub const MAX_INFLATE_BYTES: usize = 2 * 1024 * 1024;

/// Output encoding for `ParamsEncryptor::encode_aes_utf8_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AesOutputEncoding {
    Hex,
    Base64,
}

/// Exact port of `zca-js`'s `ParamsEncryptor` (used for `/api/login/getLoginInfo` and `/api/login/getServerInfo`).
#[derive(Debug, Clone)]
pub struct ParamsEncryptor {
    pub zcid: String,
    pub zcid_ext: String,
    pub enc_ver: String,
    pub encrypt_key: String,
}

impl ParamsEncryptor {
    pub fn new(api_type: u32, imei: &str, first_launch_time: i64) -> Result<Self> {
        let zcid_ext = Self::random_string(6, 12);
        Self::with_zcid_ext(api_type, imei, first_launch_time, zcid_ext)
    }

    /// Deterministic constructor useful for testing against `zca-js` vectors.
    pub fn with_zcid_ext(
        api_type: u32,
        imei: &str,
        first_launch_time: i64,
        zcid_ext: String,
    ) -> Result<Self> {
        if api_type == 0 || imei.is_empty() || first_launch_time == 0 {
            return Err(ProtoError::InvalidArg("createZcid: missing params".into()));
        }
        let msg = format!("{api_type},{imei},{first_launch_time}");
        let zcid = encode_aes_utf8_key(ZCID_KEY, &msg, AesOutputEncoding::Hex, true)?;
        let encrypt_key = derive_encrypt_key(&zcid, &zcid_ext)?;
        Ok(Self {
            zcid,
            zcid_ext,
            enc_ver: "v2".to_string(),
            encrypt_key,
        })
    }

    pub fn encrypt_key(&self) -> &str {
        &self.encrypt_key
    }

    /// Returns `{ zcid, zcid_ext, enc_ver }`.
    pub fn get_params(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("zcid".to_string(), self.zcid.clone());
        m.insert("zcid_ext".to_string(), self.zcid_ext.clone());
        m.insert("enc_ver".to_string(), self.enc_ver.clone());
        m
    }

    /// Generates a random lowercase hex string of length in `[min_len, max_len]` (defaults 6..=12).
    pub fn random_string(min_len: usize, max_len: usize) -> String {
        let min_l = if min_len == 0 { 6 } else { min_len };
        let max_l = if max_len > min_l { max_len } else { 12 };
        let mut rng = rand::thread_rng();
        let len = rng.gen_range(min_l..=max_l);
        const HEX: &[u8] = b"0123456789abcdef";
        (0..len)
            .map(|_| HEX[rng.gen_range(0..16)] as char)
            .collect()
    }
}

fn process_str(s: &str) -> (Vec<char>, Vec<char>) {
    let mut even = Vec::with_capacity(s.len().div_ceil(2));
    let mut odd = Vec::with_capacity(s.len() / 2);
    for (i, ch) in s.chars().enumerate() {
        if i % 2 == 0 {
            even.push(ch);
        } else {
            odd.push(ch);
        }
    }
    (even, odd)
}

fn derive_encrypt_key(zcid: &str, zcid_ext: &str) -> Result<String> {
    let md5_upper = md5_hex(zcid_ext.as_bytes()).to_ascii_uppercase();
    let (md5_even, _) = process_str(&md5_upper);
    let (zcid_even, mut zcid_odd) = process_str(zcid);
    if md5_even.len() < 8 || zcid_even.len() < 12 || zcid_odd.len() < 12 {
        return Err(ProtoError::Crypto(
            "Insufficient characters to derive ParamsEncryptor key".into(),
        ));
    }
    zcid_odd.reverse();
    let mut key = String::with_capacity(32);
    for c in &md5_even[..8] {
        key.push(*c);
    }
    for c in &zcid_even[..12] {
        key.push(*c);
    }
    for c in &zcid_odd[..12] {
        key.push(*c);
    }
    Ok(key)
}

/// Computes `MD5("zsecure" + kind + values_in_sorted_key_order)` matching `zca-js`'s `getSignKey`.
pub fn get_sign_key<K, V, I>(kind: &str, params: I) -> String
where
    K: AsRef<str>,
    V: AsRef<str>,
    I: IntoIterator<Item = (K, V)>,
{
    let mut sorted: Vec<(String, String)> = params
        .into_iter()
        .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut acc = String::from("zsecure");
    acc.push_str(kind);
    for (_, v) in sorted {
        acc.push_str(&v);
    }
    md5_hex(acc.as_bytes())
}

/// AES-CBC encrypt using raw UTF-8 key bytes (16/24/32 bytes), zero IV, PKCS#7.
pub fn encode_aes_utf8_key(
    key_utf8: &str,
    plaintext: &str,
    encoding: AesOutputEncoding,
    uppercase: bool,
) -> Result<String> {
    let ct = aes_cbc_encrypt_raw(key_utf8.as_bytes(), plaintext.as_bytes())?;
    let mut out = match encoding {
        AesOutputEncoding::Hex => hex::encode(ct),
        AesOutputEncoding::Base64 => B64.encode(ct),
    };
    if uppercase {
        out.make_ascii_uppercase();
    }
    Ok(out)
}

/// AES-CBC decrypt using raw UTF-8 key bytes (16/24/32 bytes), zero IV, PKCS#7, URL-decoding input first.
pub fn decode_aes_utf8_key(key_utf8: &str, encoded: &str) -> Result<String> {
    let decoded_url: Cow<'_, str> = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .unwrap_or(Cow::Borrowed(encoded));
    let ct = B64
        .decode(decoded_url.trim())
        .map_err(|e| ProtoError::Crypto(format!("base64 decode failed: {e}")))?;
    let pt = aes_cbc_decrypt_raw(key_utf8.as_bytes(), &ct)?;
    String::from_utf8(pt).map_err(|e| ProtoError::Crypto(format!("invalid UTF-8: {e}")))
}

/// Standard Zalo API AES-CBC encrypt: base64-encoded 16/24/32-byte key, zero IV, PKCS#7, base64 output.
pub fn encode_aes(secret_key_b64: &str, plaintext: &str) -> Result<String> {
    let key = B64
        .decode(secret_key_b64.trim())
        .map_err(|e| ProtoError::Crypto(format!("invalid base64 secret_key: {e}")))?;
    let ct = aes_cbc_encrypt_raw(&key, plaintext.as_bytes())?;
    Ok(B64.encode(ct))
}

/// Standard Zalo API AES-CBC decrypt: base64-encoded 16/24/32-byte key, zero IV, PKCS#7, URL-decoded base64 input.
/// Retries URL-decoding up to 4 times (matching `zca-js`'s `decodeAES` and `src/historysync.ts:decodeZaloAes`).
pub fn decode_aes(secret_key_b64: &str, encoded: &str) -> Result<String> {
    let key = B64
        .decode(secret_key_b64.trim())
        .map_err(|e| ProtoError::Crypto(format!("invalid base64 secret_key: {e}")))?;
    let mut candidate: Cow<'_, str> = Cow::Borrowed(encoded);
    let mut last_err = ProtoError::Crypto("empty ciphertext".into());
    for attempt in 0..4 {
        let changed = match percent_encoding::percent_decode_str(&candidate).decode_utf8() {
            Ok(Cow::Owned(s)) => {
                let diff = s != candidate.as_ref();
                candidate = Cow::Owned(s);
                diff
            }
            Ok(Cow::Borrowed(_)) | Err(_) => false,
        };
        if attempt > 0 && !changed {
            break;
        }
        match B64.decode(candidate.trim()) {
            Ok(ct) => match aes_cbc_decrypt_raw(&key, &ct) {
                Ok(pt) => {
                    return String::from_utf8(pt)
                        .map_err(|e| ProtoError::Crypto(format!("invalid UTF-8: {e}")));
                }
                Err(e) => last_err = e,
            },
            Err(e) => {
                last_err = ProtoError::Crypto(format!("base64 decode failed: {e}"));
            }
        }
        if !changed {
            break;
        }
    }
    Err(last_err)
}

fn aes_cbc_encrypt_raw(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    match key.len() {
        16 => Ok(Aes128CbcEnc::new_from_slices(key, &ZERO_IV)
            .map_err(|e| ProtoError::Crypto(e.to_string()))?
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext)),
        24 => Ok(Aes192CbcEnc::new_from_slices(key, &ZERO_IV)
            .map_err(|e| ProtoError::Crypto(e.to_string()))?
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext)),
        32 => Ok(Aes256CbcEnc::new_from_slices(key, &ZERO_IV)
            .map_err(|e| ProtoError::Crypto(e.to_string()))?
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext)),
        n => Err(ProtoError::Crypto(format!(
            "unsupported AES key length: {n} bytes"
        ))),
    }
}

fn aes_cbc_decrypt_raw(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return Err(ProtoError::Crypto(format!(
            "invalid AES-CBC ciphertext length: {}",
            ciphertext.len()
        )));
    }
    match key.len() {
        16 => Aes128CbcDec::new_from_slices(key, &ZERO_IV)
            .map_err(|e| ProtoError::Crypto(e.to_string()))?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|e| ProtoError::Crypto(format!("AES-128-CBC unpad failed: {e}"))),
        24 => Aes192CbcDec::new_from_slices(key, &ZERO_IV)
            .map_err(|e| ProtoError::Crypto(e.to_string()))?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|e| ProtoError::Crypto(format!("AES-192-CBC unpad failed: {e}"))),
        32 => Aes256CbcDec::new_from_slices(key, &ZERO_IV)
            .map_err(|e| ProtoError::Crypto(e.to_string()))?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|e| ProtoError::Crypto(format!("AES-256-CBC unpad failed: {e}"))),
        n => Err(ProtoError::Crypto(format!(
            "unsupported AES key length: {n} bytes"
        ))),
    }
}

/// Decodes a Zalo WebSocket event payload `{ encrypt, data }` using a pre-built [`EventCipher`]:
/// - `0`: BigInt-safe JSON parse
/// - `1`: base64 -> inflate (zlib/gzip/raw-deflate, 2 MiB max cap) -> BigInt-safe JSON parse
/// - `2`: url-decode -> base64 -> AES-{128,192,256}-GCM (16-byte nonce, 16-byte AAD, 16-byte tag) -> inflate -> BigInt-safe JSON parse
/// - `3`: same AES-GCM without inflate -> BigInt-safe JSON parse
pub fn decode_event_payload_with_cipher(
    encrypt: u8,
    data: &str,
    cipher: Option<&EventCipher>,
) -> Result<Value> {
    decode_event_payload_inner(encrypt, data, cipher, None)
}

/// Decodes a Zalo WebSocket event payload `{ encrypt, data }` given a base64 `cipher_key`.
pub fn decode_event_payload(
    encrypt: u8,
    data: &str,
    cipher_key_b64: Option<&str>,
) -> Result<Value> {
    decode_event_payload_inner(encrypt, data, None, cipher_key_b64)
}

fn decode_event_payload_inner(
    encrypt: u8,
    data: &str,
    cached_cipher: Option<&EventCipher>,
    cipher_key_b64: Option<&str>,
) -> Result<Value> {
    if encrypt == 0 {
        return parse_json_keeping_bigints(data);
    }
    if encrypt > 3 {
        return Err(ProtoError::Crypto(format!(
            "invalid encrypt type: {encrypt}"
        )));
    }

    let b64_str: Cow<'_, str> = if encrypt == 1 {
        Cow::Borrowed(data)
    } else {
        percent_encoding::percent_decode_str(data)
            .decode_utf8()
            .unwrap_or(Cow::Borrowed(data))
    };
    let buf = B64
        .decode(b64_str.trim())
        .map_err(|e| ProtoError::Crypto(format!("event base64 decode failed: {e}")))?;

    let plain = if encrypt == 1 {
        buf
    } else {
        if cached_cipher.is_none() && cipher_key_b64.is_none() {
            return Err(ProtoError::Crypto(
                "missing cipher key for encrypted event".into(),
            ));
        }
        if buf.len() < 48 {
            return Err(ProtoError::Crypto(format!(
                "encrypted event payload too short: {} bytes",
                buf.len()
            )));
        }
        let nonce: &[u8; 16] = buf[0..16].try_into().unwrap();
        let aad = &buf[16..32];
        let msg = &buf[32..];
        if let Some(cipher) = cached_cipher {
            cipher.decrypt(nonce, aad, msg)?
        } else {
            let key_b64 = cipher_key_b64.unwrap();
            let cipher = EventCipher::from_base64_key(key_b64)?;
            cipher.decrypt(nonce, aad, msg)?
        }
    };

    let bytes = if encrypt == 3 {
        plain
    } else {
        inflate_any(&plain, MAX_INFLATE_BYTES)?
    };
    let text = String::from_utf8(bytes)
        .map_err(|e| ProtoError::Crypto(format!("event payload invalid UTF-8: {e}")))?;
    parse_json_keeping_bigints(&text)
}

/// Decompresses zlib, gzip, or raw deflate bytes, enforcing a strict output byte limit.
pub fn inflate_any(buf: &[u8], max_bytes: usize) -> Result<Vec<u8>> {
    if buf.is_empty() {
        return Ok(Vec::new());
    }
    let is_gzip = buf.len() >= 2 && buf[0] == 0x1f && buf[1] == 0x8b;
    if is_gzip {
        if let Ok(out) = read_limited(GzDecoder::new(buf), max_bytes) {
            return Ok(out);
        }
    } else if let Ok(out) = read_limited(ZlibDecoder::new(buf), max_bytes) {
        return Ok(out);
    }

    // Try the alternate headered format if auto-detection missed, then raw deflate.
    if !is_gzip {
        if let Ok(out) = read_limited(GzDecoder::new(buf), max_bytes) {
            return Ok(out);
        }
    } else if let Ok(out) = read_limited(ZlibDecoder::new(buf), max_bytes) {
        return Ok(out);
    }

    read_limited(DeflateDecoder::new(buf), max_bytes).map_err(|e| {
        let head = hex::encode(&buf[..buf.len().min(8)]);
        ProtoError::Crypto(format!(
            "inflate failed ({} bytes, head={head}): {e}",
            buf.len()
        ))
    })
}

fn read_limited<R: Read>(mut reader: R, max_bytes: usize) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > max_bytes {
            return Err(format!("decompressed size exceeds limit of {max_bytes} bytes"));
        }
        out.extend_from_slice(&chunk[..n]);
    }
    Ok(out)
}

/// Parses JSON while converting any unquoted integer token with >= 16 digits outside strings into a JSON string,
/// preserving full precision for Zalo's 19-digit IDs (exact port of `src/calls.ts:parseJsonKeepingBigInts`).
pub fn parse_json_keeping_bigints(text: &str) -> Result<Value> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut in_string = false;
    let mut out: Option<String> = None;
    let mut chunk_start = 0usize;

    while i < len {
        let b = bytes[i];
        if in_string {
            if b == b'\\' {
                // Skip backslash and the following byte (all JSON escapes are ASCII;
                // UTF-8 continuation bytes never match '"' or '\\').
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if b == b'"' {
            in_string = true;
            i += 1;
            continue;
        }

        if b == b'-' || b.is_ascii_digit() {
            let start = i;
            i += 1;
            let mut all_digits = true;
            while i < len
                && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            {
                if !bytes[i].is_ascii_digit() {
                    all_digits = false;
                }
                i += 1;
            }
            let digit_start = if b == b'-' { start + 1 } else { start };
            if i - digit_start >= 16 && all_digits {
                let buf = out.get_or_insert_with(|| String::with_capacity(len + 32));
                buf.push_str(&text[chunk_start..start]);
                buf.push('"');
                buf.push_str(&text[start..i]);
                buf.push('"');
                chunk_start = i;
            }
            continue;
        }

        i += 1;
    }

    match out {
        None => Ok(serde_json::from_str(text)?),
        Some(mut buf) => {
            buf.push_str(&text[chunk_start..]);
            Ok(serde_json::from_str(&buf)?)
        }
    }
}

/// Generates a Zalo IMEI UUID: `"{uuid_v4}-{md5_hex(user_agent)}"`.
pub fn generate_zalo_uuid(user_agent: &str) -> String {
    format!("{}-{}", Uuid::new_v4(), md5_hex(user_agent.as_bytes()))
}

/// Computes lowercase hex MD5 digest of `data`.
pub fn md5_hex(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_encryptor_matches_zca_js_vectors() {
        let pe = ParamsEncryptor::with_zcid_ext(
            30,
            "test-imei-123",
            1700000000000,
            "abcdef1234".to_string(),
        )
        .unwrap();

        assert_eq!(
            pe.zcid,
            "4F64BD29470A9DC69C4ABD49735481C384A57BBF296C6539A6B7DEF28CD59BBF"
        );
        assert_eq!(pe.encrypt_key(), "9B9C3E1346B2409C94B4FB5C2E7695C9");

        let enc = encode_aes_utf8_key(
            pe.encrypt_key(),
            r#"{"computer_name":"Web","imei":"test-imei-123"}"#,
            AesOutputEncoding::Base64,
            false,
        )
        .unwrap();
        assert_eq!(
            enc,
            "LMNQUn6f1MKcSHntU6SaQEEJ7ziiYNA3wsiIX0tDoqVmm2jukDW7TElTEZQ22kca"
        );
        let dec = decode_aes_utf8_key(pe.encrypt_key(), &enc).unwrap();
        assert_eq!(dec, r#"{"computer_name":"Web","imei":"test-imei-123"}"#);

        let signkey = get_sign_key(
            "getlogininfo",
            [
                ("zcid", pe.zcid.as_str()),
                ("zcid_ext", pe.zcid_ext.as_str()),
                ("enc_ver", "v2"),
                ("params", enc.as_str()),
                ("type", "30"),
                ("client_version", "685"),
            ],
        );
        assert_eq!(signkey, "ea0424bf8ff019d4a5519b15138e981b");

        let signkey_server = get_sign_key(
            "getserverinfo",
            [
                ("imei", "test-imei-123"),
                ("type", "30"),
                ("client_version", "685"),
                ("computer_name", "Web"),
            ],
        );
        assert_eq!(signkey_server, "1ca7f306bd293a5752a1050a70765710");
    }

    #[test]
    fn encode_decode_aes_128_192_256() {
        let key16 = B64.encode(b"0123456789abcdef");
        let key24 = B64.encode(b"0123456789abcdef01234567");
        let key32 = B64.encode(b"0123456789abcdef0123456789abcdef");
        let pt = r#"{"hello":"world"}"#;

        let e16 = encode_aes(&key16, pt).unwrap();
        let e24 = encode_aes(&key24, pt).unwrap();
        let e32 = encode_aes(&key32, pt).unwrap();

        assert_eq!(e16, "55opPxURr/EHyU+oyAZyttkBQAd9j/Mv3hI46EAYntg=");
        assert_eq!(e24, "uqBnJQY/mYVEgxDviwDmSFJGJCbLfduWjWfb+YFVEU8=");
        assert_eq!(e32, "wuXfTmtFSR/KcsPsrATX3Uky7MDKQ4NpKX2/Rodrtn4=");

        // Including URL-encoded ciphertext on decode
        let enc_e16 =
            percent_encoding::utf8_percent_encode(&e16, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        let enc_e32 =
            percent_encoding::utf8_percent_encode(&e32, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        assert_eq!(decode_aes(&key16, &enc_e16).unwrap(), pt);
        assert_eq!(decode_aes(&key24, &e24).unwrap(), pt);
        assert_eq!(decode_aes(&key32, &enc_e32).unwrap(), pt);
    }

    #[test]
    fn decode_event_payload_encrypt_0_1_2_3() {
        let raw_json = r#"{"data":{"msgId":1234567890123456789,"text":"xin chao"}}"#;
        let key32 = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
        let key24 = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3";
        let key16 = "MDEyMzQ1Njc4OWFiY2RlZg==";

        // encrypt = 0
        let v0 = decode_event_payload(0, raw_json, None).unwrap();
        assert_eq!(v0["data"]["msgId"], "1234567890123456789");
        assert_eq!(v0["data"]["text"], "xin chao");

        // encrypt = 1 (zlib, gzip, raw deflate)
        let enc1_zlib = "eJyrVkpJLElUsqpWyi1O90xRsjI0MjYxNTO3sDSAs3SUSlIrSpSslCoy8xSSMxLzlWprAcGDECI=";
        let enc1_gzip = "H4sIAAAAAAAAA6tWSkksSVSyqlbKLU73TFGyMjQyNjE1M7ewNICzdJRKUitKlKyUKjLzFJIzEvOVamsB1MHhDTgAAAA=";
        let enc1_raw = "q1ZKSSxJVLKqVsotTvdMUbIyNDI2MTUzt7A0gLN0lEpSK0qUrJQqMvMUkjMS85VqawE=";
        for payload in [enc1_zlib, enc1_gzip, enc1_raw] {
            let v1 = decode_event_payload(1, payload, None).unwrap();
            assert_eq!(v1["data"]["msgId"], "1234567890123456789");
            assert_eq!(v1["data"]["text"], "xin chao");
        }

        // encrypt = 2 (AES-256-GCM 16B IV + gzip, AES-192-GCM 16B IV + zlib)
        let enc2 = "ABEiM0RVZneImaq7zN3u%2F6q7zN3u%2FwARIjNEVWZ3iJndlFlTv4XP2%2FX2wscdSsJItmlTbs%2FfQmhvQm%2FIzvCaci3O6ylc2ZOgLR65sNt3oGtPLM12q1r3s7q17Ow8V3QHGRPxvbkaPbNRgN9K7zR%2BLhmzevw%3D";
        let v2 = decode_event_payload(2, enc2, Some(key32)).unwrap();
        assert_eq!(v2["data"]["msgId"], "1234567890123456789");
        assert_eq!(v2["data"]["text"], "xin chao");

        let cipher32 = EventCipher::from_base64_key(key32).unwrap();
        let v2_cached = decode_event_payload_with_cipher(2, enc2, Some(&cipher32)).unwrap();
        assert_eq!(v2_cached, v2);

        let enc2_24 = "ABEiM0RVZneImaq7zN3u%2F6q7zN3u%2FwARIjNEVWZ3iJkC1brCNujZZRT3O1cuviuwO9zAUO%2BRgfCFyDyj8gxg%2BUanrZEz8HQ%2B90Qo%2B%2F7zUaw9HMFYE%2F%2BovaorldpyOVnpBvCYeEdY9a0%3D";
        let v2_24 = decode_event_payload(2, enc2_24, Some(key24)).unwrap();
        assert_eq!(v2_24["data"]["msgId"], "1234567890123456789");

        // encrypt = 3 (AES-128-GCM 16B IV, no inflate)
        let enc3 = "ABEiM0RVZneImaq7zN3u%2F6q7zN3u%2FwARIjNEVWZ3iJmIvSnVpq%2BKiCNPBHrsoPDof%2BE%2B6q1iDrY2ffkyJUDYfr2Uf2r3wtyPCG9KuOnaXc9BPpwjkrp57FkxGD37faYCLCkq0Xszifg%3D";
        let v3 = decode_event_payload(3, enc3, Some(key16)).unwrap();
        assert_eq!(v3["data"]["msgId"], "1234567890123456789");
        assert_eq!(v3["data"]["text"], "xin chao");
    }

    #[test]
    fn parse_json_keeping_bigints_preserves_long_ids_and_strings() {
        let input = r#"{"short":12345,"fifteen":123456789012345,"sixteen":1234567890123456,"nineteen":7234567890123456789,"neg":-9007199254740993,"float":1234567890123456.5,"str":"1234567890123456789 \"quoted\"","nested":"{\"id\":9999999999999999999}"}"#;
        let v = parse_json_keeping_bigints(input).unwrap();
        assert_eq!(v["short"], 12345);
        assert_eq!(v["fifteen"], 123456789012345i64);
        assert_eq!(v["sixteen"], "1234567890123456");
        assert_eq!(v["nineteen"], "7234567890123456789");
        assert_eq!(v["neg"], "-9007199254740993");
        assert!(v["float"].is_f64());
        assert_eq!(v["str"], "1234567890123456789 \"quoted\"");
        assert_eq!(v["nested"], r#"{"id":9999999999999999999}"#);

        // Fast-path with no >=16-digit unquoted integers (zero-allocation direct serde parse)
        let fast = r#"{"a":123,"b":"xin chào \"1234567890123456789\"","c":123456789012345}"#;
        let vf = parse_json_keeping_bigints(fast).unwrap();
        assert_eq!(vf["a"], 123);
        assert_eq!(vf["b"], "xin chào \"1234567890123456789\"");
        assert_eq!(vf["c"], 123456789012345i64);
    }

    #[test]
    fn generate_zalo_uuid_format() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0";
        let imei = generate_zalo_uuid(ua);
        let expected_suffix = format!("-{}", md5_hex(ua.as_bytes()));
        assert!(imei.ends_with(&expected_suffix));
        // UUID v4 is 36 chars + '-' + 32 hex md5 = 69 chars
        assert_eq!(imei.len(), 69);
    }
}
