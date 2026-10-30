//! Unit tests for the developer tool commands.

use oasis_types::error::Result;

use super::*;
use crate::test_helpers::assert_text;
use crate::{CommandOutput, CommandRegistry, Environment};
use oasis_vfs::{MemoryVfs, Vfs};

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
fn base64_encode() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    assert_eq!(
        assert_text!(exec(&reg, &mut vfs, "base64 hello").unwrap()),
        "aGVsbG8="
    );
}

#[test]
fn base64_decode() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    assert_eq!(
        assert_text!(exec(&reg, &mut vfs, "base64 -d aGVsbG8=").unwrap()),
        "hello"
    );
}

#[test]
fn json_validate_valid() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    assert!(
        assert_text!(exec(&reg, &mut vfs, r#"json validate {"key":"value"}"#).unwrap())
            .contains("Valid")
    );
}

#[test]
fn json_validate_invalid() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    assert!(exec(&reg, &mut vfs, "json validate {bad").is_err());
}

#[test]
fn uuid_format() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    let s = assert_text!(exec(&reg, &mut vfs, "uuid").unwrap());
    assert_eq!(s.len(), 36);
    assert_eq!(s.matches('-').count(), 4);
}

#[test]
fn seq_basic() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    let s = assert_text!(exec(&reg, &mut vfs, "seq 5").unwrap());
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, ["1", "2", "3", "4", "5"]);
}

#[test]
fn seq_range() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    let s = assert_text!(exec(&reg, &mut vfs, "seq 3 6").unwrap());
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines, ["3", "4", "5", "6"]);
}

#[test]
fn expr_arithmetic() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    assert_eq!(
        assert_text!(exec(&reg, &mut vfs, "expr 2 + 3 * 4").unwrap()),
        "14"
    );
}

#[test]
fn expr_parens() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    assert_eq!(
        assert_text!(exec(&reg, &mut vfs, "expr (2 + 3) * 4").unwrap()),
        "20"
    );
}

/// Exit status of `expr` run through `test` (0 = true), checking that
/// `test` prints nothing.
fn test_status(vfs: &mut MemoryVfs, expr: &str) -> i32 {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let out = exec(&reg, vfs, &format!("test {expr}")).unwrap();
    assert!(matches!(out, CommandOutput::None), "{out:?}");
    reg.last_exit_code()
}

#[test]
fn test_files_and_dirs() {
    let mut vfs = MemoryVfs::new();
    vfs.write("/test.txt", b"data").unwrap();
    vfs.mkdir("/tmp").unwrap();
    assert_eq!(test_status(&mut vfs, "-f /test.txt"), 0);
    assert_eq!(test_status(&mut vfs, "-f /nope.txt"), 1);
    assert_eq!(test_status(&mut vfs, "-f /tmp"), 1);
    assert_eq!(test_status(&mut vfs, "-d /tmp"), 0);
    assert_eq!(test_status(&mut vfs, "-e /tmp"), 0);
    assert_eq!(test_status(&mut vfs, "! -e /tmp"), 1);
}

#[test]
fn test_strings_and_numbers() {
    let mut vfs = MemoryVfs::new();
    assert_eq!(test_status(&mut vfs, "foo = foo"), 0);
    assert_eq!(test_status(&mut vfs, "foo != foo"), 1);
    assert_eq!(test_status(&mut vfs, "3 -lt 5"), 0);
    assert_eq!(test_status(&mut vfs, "5 -le 5"), 0);
    assert_eq!(test_status(&mut vfs, "5 -gt 5"), 1);
    assert_eq!(test_status(&mut vfs, "-z ''"), 0);
    assert_eq!(test_status(&mut vfs, "-n ''"), 1);
    assert_eq!(test_status(&mut vfs, "''"), 1);
    assert_eq!(test_status(&mut vfs, ""), 1);
}

#[test]
fn test_rejects_bad_expressions() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    assert!(exec(&reg, &mut vfs, "test x -lt 3").is_err());
    assert!(exec(&reg, &mut vfs, "test a -wat b").is_err());
    assert!(exec(&reg, &mut vfs, "[ a = a").is_err());
}

#[test]
fn bracket_form() {
    let mut reg = CommandRegistry::new();
    register_dev_commands(&mut reg);
    let mut vfs = MemoryVfs::new();
    exec(&reg, &mut vfs, "[ a = a ]").unwrap();
    assert_eq!(reg.last_exit_code(), 0);
    exec(&reg, &mut vfs, "[ a = b ]").unwrap();
    assert_eq!(reg.last_exit_code(), 1);
}
