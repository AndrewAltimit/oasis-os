//! HPACK (RFC 7541) header compression for HTTP/2.
//!
//! Minimal but complete implementation: static + dynamic table, integer
//! and string primitives, Huffman decoding, and all five representation
//! types. The encoder is deliberately simple — all literal headers are
//! emitted with *without indexing* and raw (non-Huffman) strings, which
//! every compliant HTTP/2 server must accept. The decoder must handle
//! everything the server might send, including Huffman-encoded literals
//! and dynamic table entries.

use oasis_types::error::{OasisError, Result};

// ---------------------------------------------------------------------
// Static table — RFC 7541 Appendix A
// ---------------------------------------------------------------------

/// HPACK static table (61 entries). Indices are 1-based externally
/// but stored 0-based here; the decoder adds 1 when matching.
static STATIC_TABLE: &[(&str, &str)] = &[
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

const STATIC_LEN: usize = 61;

// ---------------------------------------------------------------------
// Dynamic table
// ---------------------------------------------------------------------

/// Dynamic table entry cost (RFC 7541 §4.1): the name length + value
/// length + 32-byte fixed overhead.
fn entry_size(name: &str, value: &str) -> usize {
    name.len() + value.len() + 32
}

#[derive(Debug, Clone)]
struct DynEntry {
    name: String,
    value: String,
}

/// HPACK dynamic table. Entries are held newest-first (`entries[0]` is
/// the most recent insertion) so that lookup by HPACK index is a direct
/// array access after subtracting the static-table offset.
#[derive(Debug, Clone)]
pub struct DynamicTable {
    entries: std::collections::VecDeque<DynEntry>,
    size: usize,
    max_size: usize,
}

impl DynamicTable {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
            size: 0,
            max_size,
        }
    }

    pub fn set_max_size(&mut self, max: usize) {
        self.max_size = max;
        self.evict_to_fit();
    }

    fn evict_to_fit(&mut self) {
        while self.size > self.max_size {
            match self.entries.pop_back() {
                Some(e) => self.size -= entry_size(&e.name, &e.value),
                None => {
                    self.size = 0;
                    break;
                },
            }
        }
    }

    /// Insert a new entry. If the entry itself is larger than the
    /// current `max_size` the table is emptied per RFC 7541 §4.4.
    pub fn insert(&mut self, name: String, value: String) {
        let size = entry_size(&name, &value);
        if size > self.max_size {
            self.entries.clear();
            self.size = 0;
            return;
        }
        self.entries.push_front(DynEntry { name, value });
        self.size += size;
        self.evict_to_fit();
    }

    pub fn get(&self, idx: usize) -> Option<(&str, &str)> {
        // idx is 0-based within the dynamic table.
        self.entries
            .get(idx)
            .map(|e| (e.name.as_str(), e.value.as_str()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------
// Table lookup helper: combined static + dynamic
// ---------------------------------------------------------------------

fn lookup(table: &DynamicTable, idx: usize) -> Result<(String, String)> {
    if idx == 0 {
        return Err(OasisError::Backend("HPACK index 0 is reserved".into()));
    }
    if idx <= STATIC_LEN {
        let (n, v) = STATIC_TABLE[idx - 1];
        return Ok((n.to_string(), v.to_string()));
    }
    let dyn_idx = idx - STATIC_LEN - 1;
    match table.get(dyn_idx) {
        Some((n, v)) => Ok((n.to_string(), v.to_string())),
        None => Err(OasisError::Backend(
            format!("HPACK index {idx} out of range").into(),
        )),
    }
}

// ---------------------------------------------------------------------
// Integer codec (RFC 7541 §5.1)
// ---------------------------------------------------------------------

/// Decode an HPACK integer with an `n`-bit prefix starting at `buf[*pos]`.
///
/// On entry, `*pos` points at the byte holding the prefix. The caller is
/// responsible for having already consumed the high bits that identify
/// the representation type.
pub fn decode_integer(buf: &[u8], pos: &mut usize, prefix_bits: u8) -> Result<u64> {
    if *pos >= buf.len() {
        return Err(OasisError::Backend("HPACK integer: short buffer".into()));
    }
    let mask = (1u64 << prefix_bits) - 1;
    let mut value = (buf[*pos] as u64) & mask;
    *pos += 1;

    if value < mask {
        return Ok(value);
    }

    // Continuation bytes: 7 bits each, high bit = "more follows".
    let mut m = 0u32;
    loop {
        if *pos >= buf.len() {
            return Err(OasisError::Backend(
                "HPACK integer: unterminated continuation".into(),
            ));
        }
        let b = buf[*pos];
        *pos += 1;
        if m >= 64 {
            return Err(OasisError::Backend("HPACK integer: overflow".into()));
        }
        value = value
            .checked_add(
                ((b & 0x7f) as u64)
                    .checked_shl(m)
                    .ok_or_else(|| OasisError::Backend("HPACK integer: shift overflow".into()))?,
            )
            .ok_or_else(|| OasisError::Backend("HPACK integer: add overflow".into()))?;
        if b & 0x80 == 0 {
            return Ok(value);
        }
        m += 7;
    }
}

/// Encode an HPACK integer with an `n`-bit prefix.
///
/// `high_bits` are OR'd into the top `8 - prefix_bits` bits of the
/// first byte (the representation-type marker).
pub fn encode_integer(out: &mut Vec<u8>, high_bits: u8, prefix_bits: u8, value: u64) {
    let mask = (1u64 << prefix_bits) - 1;
    if value < mask {
        out.push(high_bits | (value as u8));
        return;
    }
    out.push(high_bits | (mask as u8));
    let mut v = value - mask;
    while v >= 128 {
        out.push(((v % 128) as u8) | 0x80);
        v /= 128;
    }
    out.push(v as u8);
}

// ---------------------------------------------------------------------
// Huffman decoder (RFC 7541 Appendix B)
// ---------------------------------------------------------------------

/// (code, bit_length) for each of the 257 symbols. Symbol 256 is EOS.
static HUFFMAN_CODES: &[(u32, u8)] = &[
    (0x1ff8, 13),
    (0x7fffd8, 23),
    (0xfffffe2, 28),
    (0xfffffe3, 28),
    (0xfffffe4, 28),
    (0xfffffe5, 28),
    (0xfffffe6, 28),
    (0xfffffe7, 28),
    (0xfffffe8, 28),
    (0xffffea, 24),
    (0x3ffffffc, 30),
    (0xfffffe9, 28),
    (0xfffffea, 28),
    (0x3ffffffd, 30),
    (0xfffffeb, 28),
    (0xfffffec, 28),
    (0xfffffed, 28),
    (0xfffffee, 28),
    (0xfffffef, 28),
    (0xffffff0, 28),
    (0xffffff1, 28),
    (0xffffff2, 28),
    (0x3ffffffe, 30),
    (0xffffff3, 28),
    (0xffffff4, 28),
    (0xffffff5, 28),
    (0xffffff6, 28),
    (0xffffff7, 28),
    (0xffffff8, 28),
    (0xffffff9, 28),
    (0xffffffa, 28),
    (0xffffffb, 28),
    (0x14, 6),
    (0x3f8, 10),
    (0x3f9, 10),
    (0xffa, 12),
    (0x1ff9, 13),
    (0x15, 6),
    (0xf8, 8),
    (0x7fa, 11),
    (0x3fa, 10),
    (0x3fb, 10),
    (0xf9, 8),
    (0x7fb, 11),
    (0xfa, 8),
    (0x16, 6),
    (0x17, 6),
    (0x18, 6),
    (0x0, 5),
    (0x1, 5),
    (0x2, 5),
    (0x19, 6),
    (0x1a, 6),
    (0x1b, 6),
    (0x1c, 6),
    (0x1d, 6),
    (0x1e, 6),
    (0x1f, 6),
    (0x5c, 7),
    (0xfb, 8),
    (0x7ffc, 15),
    (0x20, 6),
    (0xffb, 12),
    (0x3fc, 10),
    (0x1ffa, 13),
    (0x21, 6),
    (0x5d, 7),
    (0x5e, 7),
    (0x5f, 7),
    (0x60, 7),
    (0x61, 7),
    (0x62, 7),
    (0x63, 7),
    (0x64, 7),
    (0x65, 7),
    (0x66, 7),
    (0x67, 7),
    (0x68, 7),
    (0x69, 7),
    (0x6a, 7),
    (0x6b, 7),
    (0x6c, 7),
    (0x6d, 7),
    (0x6e, 7),
    (0x6f, 7),
    (0x70, 7),
    (0x71, 7),
    (0x72, 7),
    (0xfc, 8),
    (0x73, 7),
    (0xfd, 8),
    (0x1ffb, 13),
    (0x7fff0, 19),
    (0x1ffc, 13),
    (0x3ffc, 14),
    (0x22, 6),
    (0x7ffd, 15),
    (0x3, 5),
    (0x23, 6),
    (0x4, 5),
    (0x24, 6),
    (0x5, 5),
    (0x25, 6),
    (0x26, 6),
    (0x27, 6),
    (0x6, 5),
    (0x74, 7),
    (0x75, 7),
    (0x28, 6),
    (0x29, 6),
    (0x2a, 6),
    (0x7, 5),
    (0x2b, 6),
    (0x76, 7),
    (0x2c, 6),
    (0x8, 5),
    (0x9, 5),
    (0x2d, 6),
    (0x77, 7),
    (0x78, 7),
    (0x79, 7),
    (0x7a, 7),
    (0x7b, 7),
    (0x7ffe, 15),
    (0x7fc, 11),
    (0x3ffd, 14),
    (0x1ffd, 13),
    (0xffffffc, 28),
    (0xfffe6, 20),
    (0x3fffd2, 22),
    (0xfffe7, 20),
    (0xfffe8, 20),
    (0x3fffd3, 22),
    (0x3fffd4, 22),
    (0x3fffd5, 22),
    (0x7fffd9, 23),
    (0x3fffd6, 22),
    (0x7fffda, 23),
    (0x7fffdb, 23),
    (0x7fffdc, 23),
    (0x7fffdd, 23),
    (0x7fffde, 23),
    (0xffffeb, 24),
    (0x7fffdf, 23),
    (0xffffec, 24),
    (0xffffed, 24),
    (0x3fffd7, 22),
    (0x7fffe0, 23),
    (0xffffee, 24),
    (0x7fffe1, 23),
    (0x7fffe2, 23),
    (0x7fffe3, 23),
    (0x7fffe4, 23),
    (0x1fffdc, 21),
    (0x3fffd8, 22),
    (0x7fffe5, 23),
    (0x3fffd9, 22),
    (0x7fffe6, 23),
    (0x7fffe7, 23),
    (0xffffef, 24),
    (0x3fffda, 22),
    (0x1fffdd, 21),
    (0xfffe9, 20),
    (0x3fffdb, 22),
    (0x3fffdc, 22),
    (0x7fffe8, 23),
    (0x7fffe9, 23),
    (0x1fffde, 21),
    (0x7fffea, 23),
    (0x3fffdd, 22),
    (0x3fffde, 22),
    (0xfffff0, 24),
    (0x1fffdf, 21),
    (0x3fffdf, 22),
    (0x7fffeb, 23),
    (0x7fffec, 23),
    (0x1fffe0, 21),
    (0x1fffe1, 21),
    (0x3fffe0, 22),
    (0x1fffe2, 21),
    (0x7fffed, 23),
    (0x3fffe1, 22),
    (0x7fffee, 23),
    (0x7fffef, 23),
    (0xfffea, 20),
    (0x3fffe2, 22),
    (0x3fffe3, 22),
    (0x3fffe4, 22),
    (0x7ffff0, 23),
    (0x3fffe5, 22),
    (0x3fffe6, 22),
    (0x7ffff1, 23),
    (0x3ffffe0, 26),
    (0x3ffffe1, 26),
    (0xfffeb, 20),
    (0x7fff1, 19),
    (0x3fffe7, 22),
    (0x7ffff2, 23),
    (0x3fffe8, 22),
    (0x1ffffec, 25),
    (0x3ffffe2, 26),
    (0x3ffffe3, 26),
    (0x3ffffe4, 26),
    (0x7ffffde, 27),
    (0x7ffffdf, 27),
    (0x3ffffe5, 26),
    (0xfffff1, 24),
    (0x1ffffed, 25),
    (0x7fff2, 19),
    (0x1fffe3, 21),
    (0x3ffffe6, 26),
    (0x7ffffe0, 27),
    (0x7ffffe1, 27),
    (0x3ffffe7, 26),
    (0x7ffffe2, 27),
    (0xfffff2, 24),
    (0x1fffe4, 21),
    (0x1fffe5, 21),
    (0x3ffffe8, 26),
    (0x3ffffe9, 26),
    (0xffffffd, 28),
    (0x7ffffe3, 27),
    (0x7ffffe4, 27),
    (0x7ffffe5, 27),
    (0xfffec, 20),
    (0xfffff3, 24),
    (0xfffed, 20),
    (0x1fffe6, 21),
    (0x3fffe9, 22),
    (0x1fffe7, 21),
    (0x1fffe8, 21),
    (0x7ffff3, 23),
    (0x3fffea, 22),
    (0x3fffeb, 22),
    (0x1ffffee, 25),
    (0x1ffffef, 25),
    (0xfffff4, 24),
    (0xfffff5, 24),
    (0x3ffffea, 26),
    (0x7ffff4, 23),
    (0x3ffffeb, 26),
    (0x7ffffe6, 27),
    (0x3ffffec, 26),
    (0x3ffffed, 26),
    (0x7ffffe7, 27),
    (0x7ffffe8, 27),
    (0x7ffffe9, 27),
    (0x7ffffea, 27),
    (0x7ffffeb, 27),
    (0xffffffe, 28),
    (0x7ffffec, 27),
    (0x7ffffed, 27),
    (0x7ffffee, 27),
    (0x7ffffef, 27),
    (0x7fffff0, 27),
    (0x3ffffee, 26),
    (0x3fffffff, 30), // EOS
];

/// Decode a Huffman-encoded byte string. Walks the implicit prefix tree
/// bit-by-bit, matching against `HUFFMAN_CODES`. The final partial byte
/// must be 0-padded with 1-bits (RFC 7541 §5.2); any other pattern is an
/// error.
pub fn huffman_decode(input: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 2);
    let mut current: u64 = 0;
    let mut bits: u8 = 0;
    let total_bits = input.len() * 8;
    let mut consumed: usize = 0;

    for &byte in input {
        current = (current << 8) | (byte as u64);
        bits += 8;

        // Try to decode symbols as long as we have at least as many
        // bits as the longest plausible code.
        while bits >= 5 {
            let mut matched = false;
            // Match from shortest (5) to longest (30) code length.
            for len in 5u8..=30 {
                if bits < len {
                    break;
                }
                let code = ((current >> (bits - len)) & ((1u64 << len) - 1)) as u32;
                // Linear scan: tables are small relative to overall cost.
                for (sym, &(c, l)) in HUFFMAN_CODES.iter().enumerate() {
                    if l == len && c == code {
                        if sym == 256 {
                            return Err(OasisError::Backend(
                                "HPACK Huffman: EOS symbol in stream".into(),
                            ));
                        }
                        out.push(sym as u8);
                        bits -= len;
                        consumed += len as usize;
                        matched = true;
                        break;
                    }
                }
                if matched {
                    break;
                }
            }
            if !matched {
                break;
            }
        }
    }

    // Whatever remains must be at most 7 bits of all-ones EOS padding.
    if bits > 7 {
        return Err(OasisError::Backend(
            "HPACK Huffman: oversized trailing padding".into(),
        ));
    }
    if bits > 0 {
        let mask = (1u64 << bits) - 1;
        if (current & mask) != mask {
            return Err(OasisError::Backend(
                "HPACK Huffman: non-EOS padding bits".into(),
            ));
        }
    }
    let _ = (total_bits, consumed);
    Ok(out)
}

// ---------------------------------------------------------------------
// String literal decode
// ---------------------------------------------------------------------

fn decode_string(buf: &[u8], pos: &mut usize) -> Result<String> {
    if *pos >= buf.len() {
        return Err(OasisError::Backend("HPACK string: short buffer".into()));
    }
    let huffman = buf[*pos] & 0x80 != 0;
    let len = decode_integer(buf, pos, 7)? as usize;
    if *pos + len > buf.len() {
        return Err(OasisError::Backend(
            "HPACK string: length overruns buffer".into(),
        ));
    }
    let raw = &buf[*pos..*pos + len];
    *pos += len;
    let bytes = if huffman {
        huffman_decode(raw)?
    } else {
        raw.to_vec()
    };
    String::from_utf8(bytes).map_err(|_| OasisError::Backend("HPACK string: non-UTF-8".into()))
}

// ---------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------

/// Decode a complete HPACK header block into (name, value) pairs.
///
/// The dynamic table is mutated in-place per the HPACK state rules:
/// indexed references with incremental indexing add to the table,
/// explicit size updates shrink it.
pub fn decode_block(
    buf: &[u8],
    table: &mut DynamicTable,
    max_table_size: usize,
) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut pos = 0;
    // Whether a Dynamic Table Size Update is still permitted at the
    // current position. Per RFC 7541 §4.2, size updates must appear
    // only at the start of a header block.
    let mut size_update_allowed = true;

    while pos < buf.len() {
        let first = buf[pos];

        if first & 0x80 != 0 {
            // 1xxxxxxx — Indexed Header Field
            size_update_allowed = false;
            let idx = decode_integer(buf, &mut pos, 7)? as usize;
            let (n, v) = lookup(table, idx)?;
            out.push((n, v));
        } else if first & 0x40 != 0 {
            // 01xxxxxx — Literal With Incremental Indexing
            size_update_allowed = false;
            let name_idx = decode_integer(buf, &mut pos, 6)? as usize;
            let name = if name_idx == 0 {
                decode_string(buf, &mut pos)?
            } else {
                lookup(table, name_idx)?.0
            };
            let value = decode_string(buf, &mut pos)?;
            table.insert(name.clone(), value.clone());
            out.push((name, value));
        } else if first & 0x20 != 0 {
            // 001xxxxx — Dynamic Table Size Update
            if !size_update_allowed {
                return Err(OasisError::Backend(
                    "HPACK: size update not at block start".into(),
                ));
            }
            let new_max = decode_integer(buf, &mut pos, 5)? as usize;
            if new_max > max_table_size {
                return Err(OasisError::Backend(
                    "HPACK: size update exceeds settings max".into(),
                ));
            }
            table.set_max_size(new_max);
        } else {
            // 0000xxxx or 0001xxxx — Literal without / never indexed.
            size_update_allowed = false;
            let name_idx = decode_integer(buf, &mut pos, 4)? as usize;
            let name = if name_idx == 0 {
                decode_string(buf, &mut pos)?
            } else {
                lookup(table, name_idx)?.0
            };
            let value = decode_string(buf, &mut pos)?;
            out.push((name, value));
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------
// Encoder (literal-without-indexing, no Huffman)
// ---------------------------------------------------------------------

/// Encode a header field using the simplest HPACK representation:
/// literal-without-indexing with raw string literals. No dynamic-table
/// insertion occurs — safe even on the very first request since the
/// server tracks its own decoder state independently.
///
/// This adds about the same number of bytes as HTTP/1.1 headers, which
/// is fine: the real win from HTTP/2 is multiplexing + CDN compatibility,
/// not compression density on the request path.
pub fn encode_literal(out: &mut Vec<u8>, name: &str, value: &str) {
    // 0000xxxx with name_index = 0 → full name literal.
    out.push(0x00);
    encode_string(out, name);
    encode_string(out, value);
}

fn encode_string(out: &mut Vec<u8>, s: &str) {
    encode_integer(out, 0, 7, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Integer codec -----------------------------------------------------

    #[test]
    fn integer_roundtrip_small() {
        // 10 fits in 5 bits (< 31).
        let mut buf = Vec::new();
        encode_integer(&mut buf, 0, 5, 10);
        assert_eq!(buf, vec![10]);
        let mut pos = 0;
        assert_eq!(decode_integer(&buf, &mut pos, 5).unwrap(), 10);
    }

    #[test]
    fn integer_roundtrip_multi_byte() {
        // RFC 7541 §5.1 example: 1337 with 5-bit prefix → 11111 10011010 00001010
        // = 0x1f 0x9a 0x0a
        let mut buf = Vec::new();
        encode_integer(&mut buf, 0, 5, 1337);
        assert_eq!(buf, vec![0x1f, 0x9a, 0x0a]);
        let mut pos = 0;
        assert_eq!(decode_integer(&buf, &mut pos, 5).unwrap(), 1337);
    }

    #[test]
    fn integer_high_bits_preserved_on_encode() {
        let mut buf = Vec::new();
        encode_integer(&mut buf, 0x80, 7, 1);
        assert_eq!(buf, vec![0x81]);
    }

    // Static / dynamic table -------------------------------------------

    #[test]
    fn static_lookup_method_get() {
        let table = DynamicTable::new(4096);
        let (n, v) = lookup(&table, 2).unwrap();
        assert_eq!(n, ":method");
        assert_eq!(v, "GET");
    }

    #[test]
    fn static_lookup_status_200() {
        let table = DynamicTable::new(4096);
        let (n, v) = lookup(&table, 8).unwrap();
        assert_eq!(n, ":status");
        assert_eq!(v, "200");
    }

    #[test]
    fn dynamic_table_insert_and_lookup() {
        let mut table = DynamicTable::new(4096);
        table.insert("x-custom".into(), "value".into());
        let (n, v) = lookup(&table, STATIC_LEN + 1).unwrap();
        assert_eq!(n, "x-custom");
        assert_eq!(v, "value");
    }

    #[test]
    fn dynamic_table_evicts_when_full() {
        let mut table = DynamicTable::new(64); // fits one 32+N+V entry
        table.insert("aaa".into(), "111".into()); // size = 38
        table.insert("bbb".into(), "222".into()); // size = 38, evicts aaa
        assert_eq!(table.len(), 1);
        let (n, _) = lookup(&table, STATIC_LEN + 1).unwrap();
        assert_eq!(n, "bbb");
    }

    #[test]
    fn dynamic_table_oversized_entry_clears_table() {
        let mut table = DynamicTable::new(64);
        table.insert("small".into(), "ok".into());
        // Entry larger than max_size: clears and drops.
        table.insert("huge".into(), "x".repeat(100));
        assert_eq!(table.len(), 0);
    }

    // Huffman ----------------------------------------------------------

    #[test]
    fn huffman_decode_www_example_com() {
        // RFC 7541 §C.4.1: "www.example.com" Huffman-encoded.
        let data = [
            0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff,
        ];
        let out = huffman_decode(&data).unwrap();
        assert_eq!(out, b"www.example.com");
    }

    #[test]
    fn huffman_decode_single_char_a() {
        // 'a' = 5-bit code 0x3 = 00011, padded with 1-bits = 0001_1111 = 0x1f.
        let out = huffman_decode(&[0x1f]).unwrap();
        assert_eq!(out, b"a");
    }

    #[test]
    fn huffman_decode_bad_padding_rejected() {
        // 'a' (5 bits) followed by 3 padding bits of 000 instead of 111.
        let out = huffman_decode(&[0x18]);
        assert!(out.is_err());
    }

    // Full HPACK decode (RFC 7541 §C examples) -------------------------

    #[test]
    fn decode_c21_literal_header_incremental_indexing() {
        // RFC 7541 §C.2.1
        let data = [
            0x40, 0x0a, b'c', b'u', b's', b't', b'o', b'm', b'-', b'k', b'e', b'y', 0x0d, b'c',
            b'u', b's', b't', b'o', b'm', b'-', b'h', b'e', b'a', b'd', b'e', b'r',
        ];
        let mut table = DynamicTable::new(4096);
        let out = decode_block(&data, &mut table, 4096).unwrap();
        assert_eq!(out, vec![("custom-key".into(), "custom-header".into())]);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn decode_c22_literal_without_indexing() {
        // RFC 7541 §C.2.2
        let data = [
            0x04, 0x0c, b'/', b's', b'a', b'm', b'p', b'l', b'e', b'/', b'p', b'a', b't', b'h',
        ];
        let mut table = DynamicTable::new(4096);
        let out = decode_block(&data, &mut table, 4096).unwrap();
        assert_eq!(out, vec![(":path".into(), "/sample/path".into())]);
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn decode_c23_indexed_header_field() {
        // RFC 7541 §C.2.4 — indexed header :method GET (index 2).
        let data = [0x82];
        let mut table = DynamicTable::new(4096);
        let out = decode_block(&data, &mut table, 4096).unwrap();
        assert_eq!(out, vec![(":method".into(), "GET".into())]);
    }

    #[test]
    fn decode_c31_first_request() {
        // RFC 7541 §C.3.1: first request in a sequence.
        //   :method: GET
        //   :scheme: http
        //   :path: /
        //   :authority: www.example.com
        let data = [
            0x82, 0x86, 0x84, 0x41, 0x0f, b'w', b'w', b'w', b'.', b'e', b'x', b'a', b'm', b'p',
            b'l', b'e', b'.', b'c', b'o', b'm',
        ];
        let mut table = DynamicTable::new(4096);
        let out = decode_block(&data, &mut table, 4096).unwrap();
        assert_eq!(
            out,
            vec![
                (":method".into(), "GET".into()),
                (":scheme".into(), "http".into()),
                (":path".into(), "/".into()),
                (":authority".into(), "www.example.com".into()),
            ]
        );
        // Dynamic table now has one entry: (:authority, www.example.com).
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn decode_c41_first_request_huffman() {
        // RFC 7541 §C.4.1: first request, Huffman-encoded authority.
        let data = [
            0x82, 0x86, 0x84, 0x41, 0x8c, 0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab,
            0x90, 0xf4, 0xff,
        ];
        let mut table = DynamicTable::new(4096);
        let out = decode_block(&data, &mut table, 4096).unwrap();
        assert_eq!(
            out,
            vec![
                (":method".into(), "GET".into()),
                (":scheme".into(), "http".into()),
                (":path".into(), "/".into()),
                (":authority".into(), "www.example.com".into()),
            ]
        );
    }

    // Encoder ----------------------------------------------------------

    #[test]
    fn encode_literal_decodes_back() {
        let mut buf = Vec::new();
        encode_literal(&mut buf, ":method", "GET");
        encode_literal(&mut buf, ":path", "/index.html");
        encode_literal(&mut buf, "user-agent", "OASIS/1.0");

        let mut table = DynamicTable::new(4096);
        let out = decode_block(&buf, &mut table, 4096).unwrap();
        assert_eq!(
            out,
            vec![
                (":method".into(), "GET".into()),
                (":path".into(), "/index.html".into()),
                ("user-agent".into(), "OASIS/1.0".into()),
            ]
        );
    }

    #[test]
    fn decode_rejects_out_of_range_index() {
        let data = [0xff, 0x7f]; // index = 127 + 0 = 127 (out of range)
        let mut table = DynamicTable::new(4096);
        let err = decode_block(&data, &mut table, 4096).unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn size_update_after_header_rejected() {
        // Indexed header, then size update — invalid ordering.
        let data = [0x82, 0x20];
        let mut table = DynamicTable::new(4096);
        let err = decode_block(&data, &mut table, 4096).unwrap_err();
        assert!(err.to_string().contains("size update"));
    }
}
