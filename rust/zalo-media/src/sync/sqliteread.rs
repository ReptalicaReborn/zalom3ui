//! Minimal pure-Rust read-only SQLite3 file reader (`src/sqliteread.ts` port).
//!
//! Reads ordinary rowid tables (interior `0x05` / leaf `0x0d` B-tree pages, overflow page
//! chains, all record serial types, UTF-8 / UTF-16LE / UTF-16BE text encodings) directly
//! from an in-memory byte slice with zero native `libsqlite3` dependency.

use std::collections::{HashMap, HashSet};

const MAX_DEPTH: usize = 40;
const MAX_ROW_BYTES: usize = 64 * 1024 * 1024;
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SqliteError(pub String);

impl SqliteError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// A column value decoded from a SQLite3 record.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A single table row keyed by column name.
pub type SqlRow = HashMap<String, SqlValue>;

/// A read table's column list and rows in rowid order.
#[derive(Debug, Clone)]
pub struct SqlTable {
    pub columns: Vec<String>,
    pub rows: Vec<SqlRow>,
}

#[derive(Debug, Clone, Copy)]
struct Header {
    page_size: usize,
    usable: usize,
    page_count: usize,
    encoding: u32,
}

/// Returns `true` if `buf` begins with a valid SQLite3 file header.
pub fn is_sqlite(buf: &[u8]) -> bool {
    buf.len() >= 100 && &buf[..16] == SQLITE_MAGIC
}

fn read_header(db: &[u8]) -> Result<Header, SqliteError> {
    if !is_sqlite(db) {
        return Err(SqliteError::new("Not a SQLite database"));
    }
    let raw_page_size = u16::from_be_bytes([db[16], db[17]]) as usize;
    let page_size = if raw_page_size == 1 {
        65536
    } else {
        raw_page_size
    };
    if !(512..=65536).contains(&page_size) || !page_size.is_power_of_two() {
        return Err(SqliteError::new("Invalid SQLite page size"));
    }
    let reserved = db[20] as usize;
    if reserved >= page_size {
        return Err(SqliteError::new("Invalid SQLite page layout"));
    }
    let usable = page_size - reserved;
    if usable < 480 {
        return Err(SqliteError::new("Invalid SQLite page layout"));
    }
    let raw_enc = u32::from_be_bytes([db[56], db[57], db[58], db[59]]);
    let encoding = if raw_enc == 0 { 1 } else { raw_enc };
    if !(1..=3).contains(&encoding) {
        return Err(SqliteError::new("Unsupported SQLite text encoding"));
    }
    Ok(Header {
        page_size,
        usable,
        page_count: db.len() / page_size,
        encoding,
    })
}

fn read_varint(buf: &[u8], pos: usize) -> Result<(u64, usize), SqliteError> {
    let mut v: u64 = 0;
    for i in 0..8 {
        if pos + i >= buf.len() {
            return Err(SqliteError::new("Truncated SQLite varint"));
        }
        let b = buf[pos + i];
        v = (v << 7) | u64::from(b & 0x7f);
        if (b & 0x80) == 0 {
            return Ok((v, pos + i + 1));
        }
    }
    if pos + 8 >= buf.len() {
        return Err(SqliteError::new("Truncated SQLite varint"));
    }
    Ok(((v << 8) | u64::from(buf[pos + 8]), pos + 9))
}

fn decode_text(bytes: &[u8], encoding: u32) -> String {
    match encoding {
        1 => String::from_utf8_lossy(bytes).into_owned(),
        2 => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        3 => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Decodes one SQLite3 record payload into column values.
pub fn parse_record(payload: &[u8], encoding: u32) -> Result<Vec<SqlValue>, SqliteError> {
    let (header_end_u64, mut p) = read_varint(payload, 0)?;
    let header_end = usize::try_from(header_end_u64)
        .map_err(|_| SqliteError::new("Corrupt SQLite record"))?;
    if header_end > payload.len() || header_end < p {
        return Err(SqliteError::new("Corrupt SQLite record"));
    }

    let mut serial_types = Vec::new();
    while p < header_end {
        let (t, next) = read_varint(payload, p)?;
        serial_types.push(t);
        p = next;
    }

    let mut values = Vec::with_capacity(serial_types.len());
    let mut at = header_end;
    for t in serial_types {
        match t {
            0 => values.push(SqlValue::Null),
            1..=6 => {
                let size = [0usize, 1, 2, 3, 4, 6, 8][t as usize];
                if at.saturating_add(size) > payload.len() {
                    return Err(SqliteError::new("Corrupt SQLite record"));
                }
                let mut v: u64 = 0;
                for &b in &payload[at..at + size] {
                    v = (v << 8) | u64::from(b);
                }
                let signed = if size < 8 && (payload[at] & 0x80) != 0 {
                    (v | (!0u64 << (size * 8))) as i64
                } else {
                    v as i64
                };
                values.push(SqlValue::Int(signed));
                at += size;
            }
            7 => {
                if at.saturating_add(8) > payload.len() {
                    return Err(SqliteError::new("Corrupt SQLite record"));
                }
                let bits = u64::from_be_bytes(payload[at..at + 8].try_into().unwrap());
                values.push(SqlValue::Float(f64::from_bits(bits)));
                at += 8;
            }
            8 => values.push(SqlValue::Int(0)),
            9 => values.push(SqlValue::Int(1)),
            10 | 11 => return Err(SqliteError::new("Corrupt SQLite record")),
            _ => {
                let size_u64 = if t % 2 == 0 {
                    (t - 12) / 2
                } else {
                    (t - 13) / 2
                };
                let size = usize::try_from(size_u64)
                    .map_err(|_| SqliteError::new("Corrupt SQLite record"))?;
                if at.saturating_add(size) > payload.len() {
                    return Err(SqliteError::new("Corrupt SQLite record"));
                }
                let bytes = &payload[at..at + size];
                if t % 2 == 0 {
                    values.push(SqlValue::Blob(bytes.to_vec()));
                } else {
                    values.push(SqlValue::Text(decode_text(bytes, encoding)));
                }
                at += size;
            }
        }
    }
    Ok(values)
}

struct Db<'a> {
    buf: &'a [u8],
    h: Header,
}

impl<'a> Db<'a> {
    fn open(buf: &'a [u8]) -> Result<Self, SqliteError> {
        let h = read_header(buf)?;
        Ok(Self { buf, h })
    }

    fn page(&self, n: u32) -> Result<&'a [u8], SqliteError> {
        let idx = usize::try_from(n).map_err(|_| SqliteError::new("SQLite page out of range"))?;
        if idx < 1 || idx > self.h.page_count {
            return Err(SqliteError::new("SQLite page out of range"));
        }
        let start = (idx - 1) * self.h.page_size;
        Ok(&self.buf[start..start + self.h.page_size])
    }

    fn payload(&self, page: &[u8], cell_at: usize, total: usize) -> Result<Vec<u8>, SqliteError> {
        let u = self.h.usable;
        let x = u - 35;
        if total <= x {
            if cell_at.saturating_add(total) > page.len() {
                return Err(SqliteError::new("Corrupt SQLite cell payload"));
            }
            return Ok(page[cell_at..cell_at + total].to_vec());
        }
        if total > MAX_ROW_BYTES {
            return Err(SqliteError::new("SQLite row too large"));
        }
        let m = ((u - 12) * 32 / 255) - 23;
        let k = m + ((total - m) % (u - 4));
        let local = if k <= x { k } else { m };
        if cell_at.saturating_add(local).saturating_add(4) > page.len() {
            return Err(SqliteError::new("Corrupt SQLite cell overflow pointer"));
        }
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&page[cell_at..cell_at + local]);
        let mut remaining = total - local;
        let mut next = u32::from_be_bytes(
            page[cell_at + local..cell_at + local + 4]
                .try_into()
                .unwrap(),
        );
        let mut seen = HashSet::new();
        while remaining > 0 {
            if next == 0 || !seen.insert(next) {
                return Err(SqliteError::new("Corrupt SQLite overflow chain"));
            }
            let pg = self.page(next)?;
            let take = remaining.min(u - 4);
            if 4 + take > pg.len() {
                return Err(SqliteError::new("Corrupt SQLite overflow page"));
            }
            out.extend_from_slice(&pg[4..4 + take]);
            remaining -= take;
            next = u32::from_be_bytes(pg[0..4].try_into().unwrap());
        }
        Ok(out)
    }

    fn collect_rows(
        &self,
        root: u32,
        depth: usize,
        out: &mut Vec<(i64, Vec<u8>)>,
    ) -> Result<(), SqliteError> {
        if depth > MAX_DEPTH {
            return Err(SqliteError::new("SQLite b-tree too deep"));
        }
        let page = self.page(root)?;
        let base = if root == 1 { 100 } else { 0 };
        if base + 8 > page.len() {
            return Err(SqliteError::new("Corrupt SQLite page header"));
        }
        let page_type = page[base];
        let cells = u16::from_be_bytes([page[base + 3], page[base + 4]]) as usize;
        match page_type {
            0x0d => {
                if base + 8 + cells * 2 > self.h.usable {
                    return Err(SqliteError::new("Corrupt SQLite cell pointer array"));
                }
                for i in 0..cells {
                    let off = base + 8 + i * 2;
                    let ptr = u16::from_be_bytes([page[off], page[off + 1]]) as usize;
                    if ptr >= self.h.usable {
                        return Err(SqliteError::new("Corrupt SQLite cell pointer"));
                    }
                    let (size_u64, after_size) = read_varint(page, ptr)?;
                    let (rowid_u64, after_rowid) = read_varint(page, after_size)?;
                    let total = usize::try_from(size_u64)
                        .map_err(|_| SqliteError::new("SQLite row too large"))?;
                    let record = self.payload(page, after_rowid, total)?;
                    out.push((rowid_u64 as i64, record));
                }
            }
            0x05 => {
                if base + 12 + cells * 2 > self.h.usable {
                    return Err(SqliteError::new("Corrupt SQLite interior cell pointer array"));
                }
                for i in 0..cells {
                    let off = base + 12 + i * 2;
                    let ptr = u16::from_be_bytes([page[off], page[off + 1]]) as usize;
                    if ptr + 4 > self.h.usable {
                        return Err(SqliteError::new("Corrupt SQLite cell pointer"));
                    }
                    let child = u32::from_be_bytes(page[ptr..ptr + 4].try_into().unwrap());
                    self.collect_rows(child, depth + 1, out)?;
                }
                let right_child =
                    u32::from_be_bytes(page[base + 8..base + 12].try_into().unwrap());
                self.collect_rows(right_child, depth + 1, out)?;
            }
            _ => return Err(SqliteError::new("Unexpected SQLite page type")),
        }
        Ok(())
    }
}

/// Parses column names from a `CREATE TABLE` statement and returns the index of the
/// `INTEGER PRIMARY KEY` (rowid alias) column if present.
pub fn parse_create_table(sql: &str) -> Result<(Vec<String>, Option<usize>), SqliteError> {
    let open = sql
        .find('(')
        .ok_or_else(|| SqliteError::new("Unreadable table definition"))?;
    let close = sql
        .rfind(')')
        .ok_or_else(|| SqliteError::new("Unreadable table definition"))?;
    if close < open {
        return Err(SqliteError::new("Unreadable table definition"));
    }
    let body = &sql[open + 1..close];
    let mut defs = Vec::new();
    let mut depth: i32 = 0;
    let mut quote: Option<char> = None;
    let mut current = String::new();

    for c in body.chars() {
        if let Some(q) = quote {
            current.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => {
                quote = Some(c);
                current.push(c);
            }
            '[' => {
                quote = Some(']');
                current.push(c);
            }
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                defs.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        defs.push(current);
    }

    let mut columns = Vec::new();
    let mut rowid_column = None;

    for def in defs {
        let text = def.trim();
        if text.is_empty() || starts_with_table_constraint(text) {
            continue;
        }
        let Some(name) = extract_column_name(text) else {
            continue;
        };
        if has_integer_primary_key(text) && !has_word_ci(text, "desc") {
            rowid_column = Some(columns.len());
        }
        columns.push(name);
    }

    Ok((columns, rowid_column))
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn starts_with_table_constraint(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for kw in ["constraint", "primary", "unique", "check", "foreign"] {
        if let Some(rest) = lower.strip_prefix(kw) {
            if rest.is_empty() || !is_word_char(rest.as_bytes()[0]) {
                return true;
            }
        }
    }
    false
}

fn extract_column_name(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let first = chars.next()?;
    let end_quote = match first {
        '"' => Some('"'),
        '`' => Some('`'),
        '\'' => Some('\''),
        '[' => Some(']'),
        _ => None,
    };
    if let Some(q) = end_quote {
        let rest = &text[first.len_utf8()..];
        let close = rest.find(q)?;
        let name = &rest[..close];
        if name.is_empty() {
            return None;
        }
        Some(name.to_string())
    } else {
        let end = text
            .find(|c: char| c.is_whitespace())
            .unwrap_or(text.len());
        let name = &text[..end];
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }
}

fn tokenize_words_ci(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            cur.push(c.to_ascii_lowercase());
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

fn has_integer_primary_key(text: &str) -> bool {
    let words = tokenize_words_ci(text);
    words
        .windows(3)
        .any(|w| w[0] == "integer" && w[1] == "primary" && w[2] == "key")
}

fn has_word_ci(text: &str, needle: &str) -> bool {
    tokenize_words_ci(text).iter().any(|w| w == needle)
}

/// Finds `table` (case-insensitive) in `sqlite_master` and returns its columns and rows in rowid order.
/// Returns `Ok(None)` when the database is valid SQLite3 but contains no such table.
pub fn read_table(buf: &[u8], table: &str) -> Result<Option<SqlTable>, SqliteError> {
    let db = Db::open(buf)?;
    let mut master_rows = Vec::new();
    db.collect_rows(1, 0, &mut master_rows)?;

    let mut found: Option<(u32, String)> = None;
    for (_, record) in master_rows {
        let r = parse_record(&record, db.h.encoding)?;
        // sqlite_master: type, name, tbl_name, rootpage, sql
        let is_table = matches!(r.first(), Some(SqlValue::Text(t)) if t == "table");
        let name_matches =
            matches!(r.get(1), Some(SqlValue::Text(n)) if n.eq_ignore_ascii_case(table));
        if is_table && name_matches {
            let root = match r.get(3) {
                Some(SqlValue::Int(n)) if *n >= 1 => *n as u32,
                _ => return Err(SqliteError::new("Invalid table root page")),
            };
            let sql = match r.get(4) {
                Some(SqlValue::Text(s)) => s.clone(),
                _ => String::new(),
            };
            found = Some((root, sql));
            break;
        }
    }

    let Some((root, sql)) = found else {
        return Ok(None);
    };

    let (columns, rowid_column) = parse_create_table(&sql)?;
    let mut raw_rows = Vec::new();
    db.collect_rows(root, 0, &mut raw_rows)?;

    let mut rows = Vec::with_capacity(raw_rows.len());
    for (rowid, record) in raw_rows {
        let values = parse_record(&record, db.h.encoding)?;
        let mut row = HashMap::with_capacity(columns.len());
        for (i, col) in columns.iter().enumerate() {
            let val = match values.get(i) {
                None | Some(SqlValue::Null) if rowid_column == Some(i) => SqlValue::Int(rowid),
                Some(v) => v.clone(),
                None => SqlValue::Null,
            };
            row.insert(col.clone(), val);
        }
        rows.push(row);
    }

    Ok(Some(SqlTable { columns, rows }))
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

    #[test]
    fn reads_small_table_sync_1001() {
        let table = read_table(&fixture("sync_1001.db"), "ChatContent")
            .unwrap()
            .expect("ChatContent table");
        assert_eq!(
            table.columns,
            vec![
                "Id",
                "GlbMsgId",
                "CliMsgId",
                "MsgType",
                "TimeStamp",
                "SenderId",
                "MsgContent",
                "TTL",
                "BinNet",
                "MsgStatus",
            ]
        );
        assert_eq!(table.rows.len(), 12);
        assert_eq!(table.rows[0].get("Id"), Some(&SqlValue::Int(1)));
        assert_eq!(
            table.rows[0].get("GlbMsgId"),
            Some(&SqlValue::Text("9001".into()))
        );
        assert_eq!(
            table.rows[0].get("TimeStamp"),
            Some(&SqlValue::Int(1700000000000))
        );
        assert_eq!(table.rows[0].get("BinNet"), Some(&SqlValue::Null));
        assert!(matches!(table.rows[2].get("BinNet"), Some(SqlValue::Blob(_))));
        assert_eq!(table.rows[9].get("CliMsgId"), Some(&SqlValue::Null));
        assert_eq!(
            table.rows[10].get("GlbMsgId"),
            Some(&SqlValue::Text("7351234567890123456".into()))
        );
        match table.rows[11].get("MsgContent") {
            Some(SqlValue::Text(s)) => {
                assert!(s.starts_with("ünï 😀 x"));
                assert_eq!(s.chars().count(), "ünï 😀 ".chars().count() + 3000);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn reads_group_table_sync_group_777() {
        let table = read_table(&fixture("sync_group_777.db"), "ChatContent")
            .unwrap()
            .expect("ChatContent table");
        assert!(!table.rows.is_empty());
    }

    #[test]
    fn reads_interior_and_overflow_pages_sync_many() {
        let table = read_table(&fixture("sync_many.db"), "chatcontent")
            .unwrap()
            .expect("chatcontent table");
        assert_eq!(table.rows.len(), 401);
        for (i, row) in table.rows.iter().enumerate() {
            assert_eq!(row.get("Id"), Some(&SqlValue::Int((i + 1) as i64)));
        }
        assert_eq!(
            table.rows[123].get("MsgContent"),
            Some(&SqlValue::Text(format!(
                "message number 123 {}",
                "lorem ipsum ".repeat(123 % 7)
            )))
        );
        let big = &table.rows[400];
        assert_eq!(
            big.get("MsgContent"),
            Some(&SqlValue::Text(format!(
                "overflow {}",
                "0123456789".repeat(700)
            )))
        );
        match big.get("BinNet") {
            Some(SqlValue::Blob(b)) => assert_eq!(b.len(), 1500),
            other => panic!("expected 1500-byte blob, got {other:?}"),
        }
    }

    #[test]
    fn missing_table_and_non_sqlite() {
        let db = fixture("sync_1001.db");
        assert!(read_table(&db, "Nope").unwrap().is_none());
        assert!(is_sqlite(&db));
        assert!(!is_sqlite(b"hello"));
        assert!(read_table(&vec![0u8; 200], "x").is_err());
    }

    #[test]
    fn truncated_db_fails() {
        let db = fixture("sync_many.db");
        assert!(read_table(&db[..512 * 6], "ChatContent").is_err());
    }

    #[test]
    fn parse_record_all_serial_types() {
        let mut payload = vec![10u8, 0, 1, 2, 3, 7, 8, 9, 17, 16];
        payload.push(0xfe); // int8 -2
        payload.extend_from_slice(&[0x01, 0x02]); // int16 258
        payload.extend_from_slice(&[0xff, 0xff, 0xff]); // int24 -1
        payload.extend_from_slice(&1.5f64.to_be_bytes());
        payload.extend_from_slice(b"ab");
        payload.extend_from_slice(&[1, 2]);

        let vals = parse_record(&payload, 1).unwrap();
        assert_eq!(vals[0], SqlValue::Null);
        assert_eq!(vals[1], SqlValue::Int(-2));
        assert_eq!(vals[2], SqlValue::Int(258));
        assert_eq!(vals[3], SqlValue::Int(-1));
        assert_eq!(vals[4], SqlValue::Float(1.5));
        assert_eq!(vals[5], SqlValue::Int(0));
        assert_eq!(vals[6], SqlValue::Int(1));
        assert_eq!(vals[7], SqlValue::Text("ab".into()));
        assert_eq!(vals[8], SqlValue::Blob(vec![1, 2]));

        // 64-bit integer beyond JS safe range
        let mut p64 = vec![2u8, 6];
        p64.extend_from_slice(&0x7351_2345_6789_0123i64.to_be_bytes());
        assert_eq!(
            parse_record(&p64, 1).unwrap()[0],
            SqlValue::Int(0x7351_2345_6789_0123)
        );
    }

    #[test]
    fn parse_create_table_quoting_and_rowid() {
        let (cols, rowid) = parse_create_table(
            r#"CREATE TABLE "T" (`a` INTEGER PRIMARY KEY AUTOINCREMENT, [b c] TEXT DEFAULT (1,2), d NUMERIC(10,2), PRIMARY KEY (a), UNIQUE (d))"#,
        )
        .unwrap();
        assert_eq!(cols, vec!["a", "b c", "d"]);
        assert_eq!(rowid, Some(0));

        let (cols2, rowid2) = parse_create_table("CREATE TABLE t (x TEXT, y INTEGER)").unwrap();
        assert_eq!(cols2, vec!["x", "y"]);
        assert_eq!(rowid2, None);
    }
}
