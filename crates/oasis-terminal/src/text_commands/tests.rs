//! Unit tests for the text processing commands.

use super::*;
use crate::test_helpers::assert_text;
use crate::{CommandOutput, CommandRegistry, Environment};
use oasis_vfs::{MemoryVfs, Vfs};

fn setup() -> (CommandRegistry, MemoryVfs) {
    let mut reg = CommandRegistry::new();
    register_text_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/tmp").unwrap();
    vfs.write("/tmp/test.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon")
        .unwrap();
    (reg, vfs)
}

fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
    let mut env = Environment {
        cwd: "/".to_string(),
        vfs,
        power: None,
        time: None,
        usb: None,
        network: None,
        tls: None,
        stdin: None,
        stderr: String::new(),
    };
    reg.execute(line, &mut env)
}

#[test]
fn head_default() {
    let (reg, mut vfs) = setup();
    let s = assert_text!(exec(&reg, &mut vfs, "head /tmp/test.txt").unwrap());
    assert!(s.contains("alpha"));
    assert!(s.contains("epsilon"));
}

#[test]
fn head_n2() {
    let (reg, mut vfs) = setup();
    let s = assert_text!(exec(&reg, &mut vfs, "head -n 2 /tmp/test.txt").unwrap());
    assert!(s.contains("alpha"));
    assert!(s.contains("beta"));
    assert!(!s.contains("gamma"));
}

#[test]
fn tail_n2() {
    let (reg, mut vfs) = setup();
    let s = assert_text!(exec(&reg, &mut vfs, "tail -n 2 /tmp/test.txt").unwrap());
    assert!(!s.contains("gamma"));
    assert!(s.contains("delta"));
    assert!(s.contains("epsilon"));
}

#[test]
fn wc_all() {
    let (reg, mut vfs) = setup();
    assert!(assert_text!(exec(&reg, &mut vfs, "wc /tmp/test.txt").unwrap()).contains("5"));
}

#[test]
fn wc_lines() {
    let (reg, mut vfs) = setup();
    assert_eq!(
        assert_text!(exec(&reg, &mut vfs, "wc -l /tmp/test.txt").unwrap()).trim(),
        "5"
    );
}

#[test]
fn grep_basic() {
    let (reg, mut vfs) = setup();
    assert_eq!(
        assert_text!(exec(&reg, &mut vfs, "grep alpha /tmp/test.txt").unwrap()).trim(),
        "alpha"
    );
}

#[test]
fn grep_case_insensitive() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/ci.txt", b"Hello\nhello\nHELLO").unwrap();
    assert_eq!(
        assert_text!(exec(&reg, &mut vfs, "grep -i hello /tmp/ci.txt").unwrap())
            .lines()
            .count(),
        3
    );
}

#[test]
fn grep_invert() {
    let (reg, mut vfs) = setup();
    let s = assert_text!(exec(&reg, &mut vfs, "grep -v alpha /tmp/test.txt").unwrap());
    assert!(!s.contains("alpha"));
    assert!(s.contains("beta"));
}

#[test]
fn grep_count() {
    let (reg, mut vfs) = setup();
    let s = assert_text!(exec(&reg, &mut vfs, "grep -c a /tmp/test.txt").unwrap());
    let n: usize = s.trim().parse().unwrap();
    assert!(n >= 3); // alpha, gamma, delta, epsilon all contain 'a'
}

#[test]
fn sort_basic() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/unsorted.txt", b"cherry\napple\nbanana")
        .unwrap();
    let s = assert_text!(exec(&reg, &mut vfs, "sort /tmp/unsorted.txt").unwrap());
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, ["apple", "banana", "cherry"]);
}

#[test]
fn sort_reverse() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/unsorted.txt", b"a\nb\nc").unwrap();
    let s = assert_text!(exec(&reg, &mut vfs, "sort -r /tmp/unsorted.txt").unwrap());
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, ["c", "b", "a"]);
}

#[test]
fn sort_numeric() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/nums.txt", b"10\n2\n1\n20").unwrap();
    let s = assert_text!(exec(&reg, &mut vfs, "sort -n /tmp/nums.txt").unwrap());
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, ["1", "2", "10", "20"]);
}

#[test]
fn uniq_basic() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/dup.txt", b"a\na\nb\nb\nb\nc").unwrap();
    let s = assert_text!(exec(&reg, &mut vfs, "uniq /tmp/dup.txt").unwrap());
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, ["a", "b", "c"]);
}

#[test]
fn uniq_count() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/dup.txt", b"x\nx\nx\ny").unwrap();
    let s = assert_text!(exec(&reg, &mut vfs, "uniq -c /tmp/dup.txt").unwrap());
    assert!(s.contains("3"));
    assert!(s.contains("x"));
    assert!(s.contains("1"));
    assert!(s.contains("y"));
}

#[test]
fn diff_identical() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/a.txt", b"hello\nworld").unwrap();
    vfs.write("/tmp/b.txt", b"hello\nworld").unwrap();
    assert!(
        assert_text!(exec(&reg, &mut vfs, "diff /tmp/a.txt /tmp/b.txt").unwrap())
            .contains("identical")
    );
}

#[test]
fn diff_different() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/a.txt", b"hello\nworld").unwrap();
    vfs.write("/tmp/b.txt", b"hello\nearth").unwrap();
    let s = assert_text!(exec(&reg, &mut vfs, "diff /tmp/a.txt /tmp/b.txt").unwrap());
    assert!(s.contains("< world"));
    assert!(s.contains("> earth"));
}

#[test]
fn cut_fields() {
    let (reg, mut vfs) = setup();
    vfs.write("/tmp/csv.txt", b"a,b,c\nd,e,f").unwrap();
    let s = assert_text!(exec(&reg, &mut vfs, "cut -d , -f 1,3 /tmp/csv.txt").unwrap());
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, ["a,c", "d,f"]);
}

#[test]
fn tr_translate() {
    let mut reg = CommandRegistry::new();
    register_text_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    let mut env = Environment {
        cwd: "/".to_string(),
        vfs: &mut vfs,
        power: None,
        time: None,
        usb: None,
        network: None,
        tls: None,
        stdin: Some("hello".to_string()),
        stderr: String::new(),
    };
    assert_eq!(
        assert_text!(reg.execute("tr elo ELO", &mut env).unwrap()),
        "hELLO"
    );
}

#[test]
fn tr_delete() {
    let mut reg = CommandRegistry::new();
    register_text_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    let mut env = Environment {
        cwd: "/".to_string(),
        vfs: &mut vfs,
        power: None,
        time: None,
        usb: None,
        network: None,
        tls: None,
        stdin: Some("hello world".to_string()),
        stderr: String::new(),
    };
    assert_eq!(
        assert_text!(reg.execute("tr -d lo", &mut env).unwrap()),
        "he wrd"
    );
}
