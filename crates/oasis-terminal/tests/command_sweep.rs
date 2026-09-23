//! Robustness sweep over every registered command.
//!
//! Every command (plus the shell builtins) is invoked with no arguments,
//! with `--help`, and with a set of hostile / nonsensical arguments. The
//! assertion is simply that nothing panics and every call returns within a
//! bounded time; errors are expected and fine.

#![allow(clippy::unwrap_used)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use oasis_terminal::{CommandRegistry, Environment, register_builtins};
use oasis_vfs::{MemoryVfs, Vfs};

fn make_registry() -> CommandRegistry {
    let mut reg = CommandRegistry::new();
    register_builtins(&mut reg);
    reg
}

fn seeded_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home/user").unwrap();
    vfs.mkdir("/tmp").unwrap();
    vfs.write("/home/user/a.txt", b"alpha\nbeta\ngamma\n")
        .unwrap();
    vfs.write("/home/user/b.bin", &[0, 159, 146, 150, 255, 0, 10])
        .unwrap();
    vfs
}

fn make_env(vfs: &mut MemoryVfs) -> Environment<'_> {
    Environment {
        cwd: "/home/user".to_string(),
        vfs,
        power: None,
        time: None,
        usb: None,
        network: None,
        tls: None,
        stdin: None,
        stderr: String::new(),
    }
}

/// Commands whose no-arg / bad-arg behaviour legitimately does network I/O
/// or waits; they are still invoked, just with arguments that cannot block.
fn is_slow_by_design(name: &str) -> bool {
    matches!(name, "sleep" | "watch" | "ping" | "curl" | "wget" | "http")
}

const BAD_ARGS: &[&str] = &[
    "--help",
    "-h",
    "--no-such-flag",
    "-",
    "--",
    "''",
    "\"\"",
    "../../../../etc/passwd",
    "/nonexistent/path/x",
    "-1",
    "99999999999999999999999999",
    "NaN",
    "%%%",
    "$UNSET_VAR",
    "\u{1F600}\u{0301}",
    "a b c d e f g h i j",
    "-n -1",
    "-n 99999999999",
    "0x",
    "=",
    "[",
    "*",
];

fn run_case(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str, failures: &mut Vec<String>) {
    let started = Instant::now();
    let mut env = make_env(vfs);
    let result = catch_unwind(AssertUnwindSafe(|| reg.execute(line, &mut env)));
    let elapsed = started.elapsed();
    if result.is_err() {
        failures.push(format!("PANIC: `{line}`"));
    }
    if elapsed > Duration::from_secs(2) {
        failures.push(format!("SLOW ({elapsed:?}): `{line}`"));
    }
}

#[test]
fn every_command_survives_no_args_help_and_bad_args() {
    let reg = make_registry();
    let mut names = reg.completion_names();
    names.sort();
    names.dedup();
    assert!(
        names.len() > 80,
        "expected the full builtin set, got {}",
        names.len()
    );

    // Failures (panics are caught) are collected and reported together.
    let mut failures = Vec::new();
    for name in &names {
        let mut vfs = seeded_vfs();
        if !is_slow_by_design(name) {
            run_case(&reg, &mut vfs, name, &mut failures);
        }
        for bad in BAD_ARGS {
            if is_slow_by_design(name) && !bad.starts_with('-') {
                continue;
            }
            run_case(&reg, &mut vfs, &format!("{name} {bad}"), &mut failures);
        }
        // Piped stdin with binary-ish content.
        if !is_slow_by_design(name) {
            run_case(
                &reg,
                &mut vfs,
                &format!("cat /home/user/b.bin | {name}"),
                &mut failures,
            );
        }
        // The VFS must still be usable after whatever the command did.
        assert!(vfs.exists("/"), "VFS root vanished after `{name}`");
    }

    assert!(
        failures.is_empty(),
        "{} failing invocations:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every command, fed path-traversal arguments against a `RealVfs` rooted in
/// a temp dir, must neither read nor modify anything outside the root.
#[test]
fn every_command_stays_inside_real_vfs_root() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!("oasis-sweep-{}-{nanos}", std::process::id()));
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(root.join("home/user")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"TOP SECRET").unwrap();
    let outside_abs = outside.display().to_string();

    let reg = make_registry();
    let mut names = reg.completion_names();
    names.sort();
    names.dedup();
    let targets = [
        "../../outside/secret.txt".to_string(),
        "../../outside/new.txt".to_string(),
        "..\\..\\outside\\secret.txt".to_string(),
        format!("{outside_abs}/secret.txt"),
        format!("{outside_abs}/new.txt"),
    ];
    let mut leaks = Vec::new();
    for name in &names {
        if is_slow_by_design(name) || name == "run" {
            continue;
        }
        let mut vfs = oasis_vfs::RealVfs::new(&root).unwrap();
        for t in &targets {
            for line in [
                format!("{name} {t}"),
                format!("{name} {t} {t}"),
                format!("{name} /home/user/x {t}"),
                format!("{name} -n 1 {t}"),
            ] {
                let mut env = Environment {
                    cwd: "/home/user".to_string(),
                    vfs: &mut vfs,
                    power: None,
                    time: None,
                    usb: None,
                    network: None,
                    tls: None,
                    stdin: None,
                    stderr: String::new(),
                };
                let out = catch_unwind(AssertUnwindSafe(|| reg.execute(&line, &mut env)));
                let text = match out {
                    Ok(Ok(o)) => format!("{o:?}"),
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => {
                        leaks.push(format!("PANIC: `{line}`"));
                        continue;
                    },
                };
                if text.contains("TOP SECRET") {
                    leaks.push(format!("READ LEAK: `{line}`"));
                }
            }
        }
    }
    let secret = std::fs::read(outside.join("secret.txt")).unwrap_or_default();
    let listing: Vec<String> = std::fs::read_dir(&outside)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let base_listing: Vec<String> = std::fs::read_dir(&base)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&base);
    assert!(leaks.is_empty(), "{leaks:#?}");
    assert_eq!(secret, b"TOP SECRET", "file outside the root was modified");
    assert_eq!(
        listing,
        vec!["secret.txt".to_string()],
        "files created outside"
    );
    let mut base_listing = base_listing;
    base_listing.sort();
    assert_eq!(base_listing, vec!["outside", "root"]);
}

#[test]
fn every_command_has_help_text() {
    let reg = make_registry();
    let mut vfs = seeded_vfs();
    let mut names = reg.completion_names();
    names.sort();
    names.dedup();
    let mut missing = Vec::new();
    for name in &names {
        let mut env = make_env(&mut vfs);
        match reg.execute(&format!("help {name}"), &mut env) {
            Ok(_) => {},
            Err(e) => {
                // Shell builtins are intercepted and not in the command map.
                let mut env = make_env(&mut vfs);
                let which = reg.execute(&format!("which {name}"), &mut env);
                if which.is_err() {
                    missing.push(format!("{name}: {e}"));
                }
            },
        }
    }
    assert!(missing.is_empty(), "no help/which for: {missing:?}");
}
