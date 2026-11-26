//! LZMA / XZ decompression for phone history-sync backups (`src/lzma.ts` port).
//!
//! Uses `lzma-rs` with a bounded writer enforcing `max_output` (default 256 MiB)
//! and handles trailing AES zero-padding after `.xz` stream footers.

use std::io::{self, Cursor, Write};

/// Default cap on decoded bytes (256 MiB, matching Zalos and `src/lzma.ts`).
pub const DEFAULT_MAX_OUTPUT: usize = 256 * 1024 * 1024;

/// Standard `.xz` container magic bytes.
pub const XZ_MAGIC: &[u8; 6] = &[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00];

#[derive(Debug, thiserror::Error)]
pub enum LzmaError {
    #[error("Decoded data is larger than the allowed maximum")]
    OutputLimitExceeded,

    #[error("{0}")]
    InvalidData(String),
}

struct BoundedWriter {
    buf: Vec<u8>,
    max: usize,
    exceeded: bool,
}

impl BoundedWriter {
    fn new(initial_cap: usize, max: usize) -> Self {
        Self {
            buf: Vec::with_capacity(initial_cap.min(max).min(4 * 1024 * 1024)),
            max,
            exceeded: false,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len().saturating_add(data.len()) > self.max {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "output limit exceeded",
            ));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Decodes a `.xz` container stream with a hard cap of `max_output` decoded bytes.
pub fn decode_xz(src: &[u8], max_output: usize) -> Result<Vec<u8>, LzmaError> {
    if src.len() < 12 || &src[..6] != XZ_MAGIC {
        return Err(LzmaError::InvalidData("Not an .xz stream".into()));
    }

    // ZDB4.0 decrypts AES-256-CBC with NoPadding, leaving 0..16 bytes of zero, PKCS#7,
    // or block-remainder padding after the .xz stream footer ("YZ"). Parse the exact
    // single-stream boundary (header + blocks + index + 12-byte footer) so trailing
    // AES padding never triggers `Unexpected data after last XZ block` in `lzma-rs`.
    let stream_end = match validate_and_find_xz_stream_end(src)? {
        Some(end) => end,
        None => find_xz_footer_fallback(src).unwrap_or(src.len()),
    };
    let trimmed = &src[..stream_end];

    let mut reader = Cursor::new(trimmed);
    let mut writer = BoundedWriter::new(64 * 1024, max_output);
    match lzma_rs::xz_decompress(&mut reader, &mut writer) {
        Ok(()) => Ok(writer.buf),
        Err(err) => {
            if writer.exceeded {
                Err(LzmaError::OutputLimitExceeded)
            } else {
                Err(LzmaError::InvalidData(format!("XZ decode failed: {err}")))
            }
        }
    }
}

fn find_xz_footer_fallback(src: &[u8]) -> Option<usize> {
    if src.len() < 24 {
        return None;
    }
    let flags = &src[6..8];
    let min_end = src.len().saturating_sub(32).max(24);
    for end in (min_end..=src.len()).rev() {
        if src[end - 2] == 0x59 && src[end - 1] == 0x5a && &src[end - 4..end - 2] == flags {
            return Some(end);
        }
    }
    None
}

fn read_xz_varint(src: &[u8], pos: usize) -> Result<(u64, usize), LzmaError> {
    let mut val = 0u64;
    for i in 0..9 {
        let b = *src
            .get(pos + i)
            .ok_or_else(|| LzmaError::InvalidData("Truncated .xz data".into()))?;
        val |= u64::from(b & 0x7f) << (7 * i);
        if (b & 0x80) == 0 {
            return Ok((val, pos + i + 1));
        }
    }
    Err(LzmaError::InvalidData("Corrupt .xz varint".into()))
}

/// Validates that compressed LZMA2 chunks in single-filter `.xz` blocks begin with the
/// spec-mandated `0x00` range-coder init byte (`lzma-rs` skips this check), and returns
/// the exact byte offset immediately after the 12-byte `.xz` stream footer (`"YZ"`).
fn validate_and_find_xz_stream_end(src: &[u8]) -> Result<Option<usize>, LzmaError> {
    let check_size = match src[7] & 0x0f {
        0 => 0usize,
        1 => 4,
        4 => 8,
        10 => 32,
        _ => return Ok(None),
    };
    let mut pos = 12usize;
    let mut saw_index = false;
    while pos < src.len() {
        let size_byte = src[pos];
        if size_byte == 0 {
            saw_index = true;
            break;
        }
        let header_size = (usize::from(size_byte) + 1) * 4;
        if pos + header_size > src.len() {
            return Err(LzmaError::InvalidData("Truncated .xz block header".into()));
        }
        let flags = src[pos + 1];
        let mut p = pos + 2;
        if (flags & 0x40) != 0 {
            p = read_xz_varint(src, p)?.1;
        }
        if (flags & 0x80) != 0 {
            p = read_xz_varint(src, p)?.1;
        }
        let filters = (flags & 0x03) + 1;
        if filters != 1 {
            return Ok(None);
        }
        let (filter_id, _) = read_xz_varint(src, p)?;
        if filter_id != 0x21 {
            return Ok(None);
        }
        pos += header_size;

        // Walk LZMA2 chunks until 0x00 end marker.
        loop {
            let control = *src
                .get(pos)
                .ok_or_else(|| LzmaError::InvalidData("Truncated LZMA2 data".into()))?;
            pos += 1;
            if control == 0 {
                break;
            }
            if control >= 0x80 {
                if pos + 4 > src.len() {
                    return Err(LzmaError::InvalidData("Truncated LZMA2 chunk header".into()));
                }
                let packed = u16::from_be_bytes([src[pos + 2], src[pos + 3]]) as usize + 1;
                pos += 4;
                if control >= 0xc0 {
                    if pos >= src.len() {
                        return Err(LzmaError::InvalidData("Truncated LZMA2 properties".into()));
                    }
                    pos += 1;
                }
                if pos + packed > src.len() {
                    return Err(LzmaError::InvalidData("Truncated LZMA2 chunk payload".into()));
                }
                if src[pos] != 0 {
                    return Err(LzmaError::InvalidData(
                        "Corrupt LZMA data (range coder start)".into(),
                    ));
                }
                pos += packed;
            } else if control == 1 || control == 2 {
                if pos + 2 > src.len() {
                    return Err(LzmaError::InvalidData("Truncated LZMA2 uncompressed chunk".into()));
                }
                let size = u16::from_be_bytes([src[pos], src[pos + 1]]) as usize + 1;
                pos += 2;
                if pos + size > src.len() {
                    return Err(LzmaError::InvalidData("Truncated LZMA2 uncompressed payload".into()));
                }
                pos += size;
            } else {
                return Err(LzmaError::InvalidData("Corrupt LZMA2 control byte".into()));
            }
        }

        let pad = (4 - (pos % 4)) % 4;
        pos = pos.saturating_add(pad).saturating_add(check_size);
    }

    if !saw_index {
        return Ok(None);
    }

    // Parse Index (0x00 indicator + record count + 2 varints per record + 0..3 pad + 4B CRC32)
    // followed by the 12-byte Stream Footer (4B CRC32 + 4B Backward Size + 2B Flags + "YZ").
    let index_start = pos;
    pos += 1; // index indicator 0x00
    let (num_records, next) = read_xz_varint(src, pos)?;
    pos = next;
    for _ in 0..num_records {
        pos = read_xz_varint(src, pos)?.1; // Unpadded Size
        pos = read_xz_varint(src, pos)?.1; // Uncompressed Size
    }
    let idx_pad = (4 - ((pos - index_start) % 4)) % 4;
    pos = pos.saturating_add(idx_pad).saturating_add(4); // Index CRC32
    let footer_end = pos.saturating_add(12);
    if footer_end <= src.len()
        && src[footer_end - 2] == 0x59
        && src[footer_end - 1] == 0x5a
        && src[footer_end - 4..footer_end - 2] == src[6..8]
    {
        Ok(Some(footer_end))
    } else {
        Ok(None)
    }
}

/// Decodes a raw `.lzma` ("lzma_alone") stream with a hard cap of `max_output` decoded bytes.
pub fn decode_lzma_alone(src: &[u8], max_output: usize) -> Result<Vec<u8>, LzmaError> {
    if src.len() < 13 + 5 {
        return Err(LzmaError::InvalidData("Truncated LZMA data".into()));
    }
    if src[0] >= 9 * 5 * 5 {
        return Err(LzmaError::InvalidData("Invalid LZMA properties".into()));
    }
    if src[13] != 0 {
        return Err(LzmaError::InvalidData(
            "Corrupt LZMA data (range coder start)".into(),
        ));
    }
    let size_u64 = u64::from_le_bytes(src[5..13].try_into().unwrap());
    let known_size = if size_u64 != u64::MAX {
        if size_u64 > max_output as u64 {
            return Err(LzmaError::OutputLimitExceeded);
        }
        Some(size_u64 as usize)
    } else {
        None
    };

    let mut reader = Cursor::new(src);
    let mut writer = BoundedWriter::new(known_size.unwrap_or(64 * 1024), max_output);
    match lzma_rs::lzma_decompress(&mut reader, &mut writer) {
        Ok(()) => {
            if let Some(expected) = known_size {
                if writer.buf.len() != expected {
                    return Err(LzmaError::InvalidData("LZMA data ended early".into()));
                }
            }
            Ok(writer.buf)
        }
        Err(err) => {
            if writer.exceeded {
                Err(LzmaError::OutputLimitExceeded)
            } else {
                Err(LzmaError::InvalidData(format!("LZMA decode failed: {err}")))
            }
        }
    }
}

/// Auto-detects `.xz` (by magic header) or `.lzma` ("lzma_alone") and decompresses with `max_output` cap.
pub fn decode_lzma_auto(src: &[u8], max_output: usize) -> Result<Vec<u8>, LzmaError> {
    if src.len() >= 6 && &src[..6] == XZ_MAGIC {
        decode_xz(src, max_output)
    } else {
        decode_lzma_alone(src, max_output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
    }

    fn lcg(n: usize, seed: u32) -> Vec<u8> {
        let mut out = vec![0u8; n];
        let mut x = seed;
        for b in &mut out {
            x = x.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fffffff;
            *b = ((x >> 16) & 0xff) as u8;
        }
        out
    }

    fn expected_text() -> Vec<u8> {
        let part = format!(
            "{}Xin chào, đây là tin nhắn thử nghiệm.\n",
            "The quick brown fox jumps over the lazy dog. ".repeat(40)
        );
        part.repeat(5).into_bytes()
    }

    #[test]
    fn xz_small_text() {
        assert_eq!(decode_xz(&fixture("lz_text.xz"), DEFAULT_MAX_OUTPUT).unwrap(), expected_text());
    }

    #[test]
    fn lzma_alone_small_text() {
        assert_eq!(
            decode_lzma_alone(&fixture("lz_text.lzma"), DEFAULT_MAX_OUTPUT).unwrap(),
            expected_text()
        );
    }

    #[test]
    fn auto_detects_xz_and_lzma() {
        let text = expected_text();
        assert_eq!(decode_lzma_auto(&fixture("lz_text.xz"), DEFAULT_MAX_OUTPUT).unwrap(), text);
        assert_eq!(decode_lzma_auto(&fixture("lz_text.lzma"), DEFAULT_MAX_OUTPUT).unwrap(), text);
    }

    #[test]
    fn xz_multi_chunk_lz_big() {
        let chunk = lcg(997, 7);
        let mut expected = Vec::with_capacity(4_500_000);
        while expected.len() < 4_500_000 {
            let take = (4_500_000 - expected.len()).min(chunk.len());
            expected.extend_from_slice(&chunk[..take]);
        }
        assert_eq!(decode_xz(&fixture("lz_big.xz"), DEFAULT_MAX_OUTPUT).unwrap(), expected);
    }

    #[test]
    fn xz_incompressible_lz_rnd() {
        assert_eq!(
            decode_xz(&fixture("lz_rnd.xz"), DEFAULT_MAX_OUTPUT).unwrap(),
            lcg(3000, 99)
        );
    }

    #[test]
    fn xz_sync_body_and_aes_zero_padding() {
        let raw = fixture("sync_body.xz");
        let expected_1001 = fixture("sync_1001.db");
        let expected_777 = fixture("sync_group_777.db");
        let mut expected = Vec::with_capacity(expected_1001.len() + expected_777.len());
        expected.extend_from_slice(&expected_1001);
        expected.extend_from_slice(&expected_777);

        let decoded = decode_lzma_auto(&raw, DEFAULT_MAX_OUTPUT).unwrap();
        assert_eq!(decoded, expected);

        // Verify all 1..=15 AES block trailing zero pad lengths work cleanly.
        for pad in 1..=15 {
            let mut padded = raw.clone();
            padded.resize(raw.len() + pad, 0);
            let dec = decode_lzma_auto(&padded, DEFAULT_MAX_OUTPUT).unwrap();
            assert_eq!(dec, expected, "failed with {pad} trailing zero bytes");
        }
    }

    #[test]
    fn xz_sync_body_with_pkcs7_and_nonzero_trailing_bytes() {
        let raw = fixture("sync_body.xz");
        let expected_1001 = fixture("sync_1001.db");
        let expected_777 = fixture("sync_group_777.db");
        let mut expected = Vec::with_capacity(expected_1001.len() + expected_777.len());
        expected.extend_from_slice(&expected_1001);
        expected.extend_from_slice(&expected_777);

        // PKCS#7 padding (1..=16 bytes of pad length value) after the YZ footer.
        for pad in 1u8..=16 {
            let mut padded = raw.clone();
            padded.extend(std::iter::repeat_n(pad, pad as usize));
            let dec = decode_lzma_auto(&padded, DEFAULT_MAX_OUTPUT).unwrap();
            assert_eq!(dec, expected, "failed with PKCS#7 pad={pad}");
        }

        // Arbitrary non-zero trailing bytes (including fake "YZ" bytes in the tail).
        let mut junk = raw.clone();
        junk.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0x59, 0x5a, 0x07, 0x07]);
        assert_eq!(decode_lzma_auto(&junk, DEFAULT_MAX_OUTPUT).unwrap(), expected);
    }

    #[test]
    fn enforces_max_output_limit() {
        assert!(matches!(
            decode_xz(&fixture("lz_big.xz"), 1_000_000),
            Err(LzmaError::OutputLimitExceeded)
        ));
        assert!(matches!(
            decode_lzma_alone(&fixture("lz_text.lzma"), 100),
            Err(LzmaError::OutputLimitExceeded)
        ));
    }

    #[test]
    fn damaged_streams_return_error() {
        let xz = fixture("lz_text.xz");
        assert!(decode_xz(&xz[..40], DEFAULT_MAX_OUTPUT).is_err());

        let mut broken = xz.clone();
        broken[30] ^= 0xff;
        if let Ok(dec) = decode_xz(&broken, DEFAULT_MAX_OUTPUT) {
            assert_ne!(dec, expected_text());
        }

        assert!(decode_lzma_auto(b"not lzma at all, just some text", DEFAULT_MAX_OUTPUT).is_err());
    }
}
