//! Calculation history persistence.
//!
//! History is stored as plain text at [`HISTORY_PATH`], one
//! `expression=result` entry per line (oldest first). Expressions never
//! contain `=`, so the last `=` on a line separates the two halves.

use oasis_vfs::Vfs;

use crate::CalcHistoryEntry;

/// Where the calculator keeps its history between sessions.
pub const HISTORY_PATH: &str = "/home/user/.calc_history";

/// Maximum number of entries kept (oldest are dropped first).
pub const MAX_HISTORY: usize = 50;

/// Serialize `entries` (the newest [`MAX_HISTORY`] of them).
pub fn serialize(entries: &[CalcHistoryEntry]) -> String {
    let start = entries.len().saturating_sub(MAX_HISTORY);
    let mut out = String::new();
    for e in &entries[start..] {
        out.push_str(&e.expression);
        out.push('=');
        out.push_str(&e.result.to_string());
        out.push('\n');
    }
    out
}

/// Parse a history file. Malformed lines are skipped.
pub fn parse(text: &str) -> Vec<CalcHistoryEntry> {
    let mut entries: Vec<CalcHistoryEntry> = text
        .lines()
        .filter_map(|line| {
            let (expr, result) = line.rsplit_once('=')?;
            let expr = expr.trim();
            if expr.is_empty() {
                return None;
            }
            Some(CalcHistoryEntry {
                expression: expr.to_string(),
                result: result.trim().parse().ok()?,
            })
        })
        .collect();
    let excess = entries.len().saturating_sub(MAX_HISTORY);
    entries.drain(..excess);
    entries
}

/// Load history from `vfs` (empty when the file is missing or unreadable).
pub fn load(vfs: &dyn Vfs) -> Vec<CalcHistoryEntry> {
    if !vfs.exists(HISTORY_PATH) {
        return Vec::new();
    }
    vfs.read(HISTORY_PATH)
        .map(|data| parse(&String::from_utf8_lossy(&data)))
        .unwrap_or_default()
}

/// Write `entries` to [`HISTORY_PATH`], creating its folder if needed.
pub fn save(vfs: &mut dyn Vfs, entries: &[CalcHistoryEntry]) -> Result<(), String> {
    let dir = HISTORY_PATH
        .rsplit_once('/')
        .map_or("/", |(d, _)| if d.is_empty() { "/" } else { d });
    let mut cur = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        if !vfs.exists(&cur) {
            vfs.mkdir(&cur).map_err(|e| e.to_string())?;
        }
    }
    vfs.write(HISTORY_PATH, serialize(entries).as_bytes())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn entry(expr: &str, result: f64) -> CalcHistoryEntry {
        CalcHistoryEntry {
            expression: expr.to_string(),
            result,
        }
    }

    #[test]
    fn serialize_parse_round_trip() {
        let entries = vec![
            entry("1+1", 2.0),
            entry("(2+3)*-4", -20.0),
            entry("1/3", 1.0 / 3.0),
        ];
        let parsed = parse(&serialize(&entries));
        assert_eq!(parsed.len(), 3);
        for (a, b) in entries.iter().zip(&parsed) {
            assert_eq!(a.expression, b.expression);
            assert_eq!(a.result, b.result);
        }
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let parsed = parse("1+1=2\ngarbage\n=5\n2*2=notanumber\n3*3=9\n");
        let exprs: Vec<_> = parsed.iter().map(|e| e.expression.as_str()).collect();
        assert_eq!(exprs, ["1+1", "3*3"]);
    }

    #[test]
    fn keeps_only_newest_entries() {
        let entries: Vec<_> = (0..MAX_HISTORY + 5)
            .map(|i| entry(&format!("{i}+0"), i as f64))
            .collect();
        let parsed = parse(&serialize(&entries));
        assert_eq!(parsed.len(), MAX_HISTORY);
        assert_eq!(parsed[0].expression, "5+0");
    }

    #[test]
    fn save_creates_folder_and_load_reads_back() {
        let mut vfs = MemoryVfs::new();
        assert!(load(&vfs).is_empty());
        save(&mut vfs, &[entry("6*7", 42.0)]).expect("save");
        let loaded = load(&vfs);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].expression, "6*7");
        assert_eq!(loaded[0].result, 42.0);
    }
}
