//! Host stack cost of shell nesting.
//!
//! Every nesting level (`$(...)` in `$(...)`, a function calling a
//! function, a compound body) recurses through `CommandRegistry::execute`,
//! so `MAX_EXEC_DEPTH` levels must fit on the smallest stack the shell runs
//! on with room to spare. These tests measure the bytes per level and pin a
//! budget for optimized builds.

use std::cell::RefCell;
use std::rc::Rc;

use oasis_terminal::{
    Command, CommandOutput, CommandRegistry, CommandResult, Environment, MAX_EXEC_DEPTH,
};
use oasis_vfs::MemoryVfs;

/// Budget for the whole nesting limit in an optimized build: 128 KiB, half
/// of the 256 KiB main-thread stack rust-psp gives a PSP EBOOT (its
/// smallest host), leaving the rest for the host's own frames.
const RELEASE_BUDGET: usize = 128 * 1024;

/// Records the address of a stack local each time it runs.
struct Probe(Rc<RefCell<Vec<usize>>>);

impl Command for Probe {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "record the stack pointer"
    }
    fn usage(&self) -> &str {
        "probe"
    }
    fn execute(&self, _args: &[&str], _env: &mut Environment<'_>) -> CommandResult {
        let marker = 0u8;
        self.0
            .borrow_mut()
            .push(std::hint::black_box(&marker) as *const u8 as usize);
        Ok(CommandOutput::None)
    }
}

fn env(vfs: &mut MemoryVfs) -> Environment<'_> {
    Environment {
        cwd: "/".to_string(),
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

/// Average stack bytes per level between `levels` and 1 level of nesting
/// built by `line(n)`.
fn bytes_per_level(line: impl Fn(usize) -> String, levels: usize) -> usize {
    let probes = Rc::new(RefCell::new(Vec::new()));
    let mut reg = CommandRegistry::new();
    oasis_terminal::register_builtins(&mut reg);
    reg.register(Box::new(Probe(Rc::clone(&probes))));
    let mut vfs = MemoryVfs::new();

    let mut sp = |n: usize| {
        probes.borrow_mut().clear();
        reg.execute(&line(n), &mut env(&mut vfs))
            .expect("nested line runs");
        // The innermost probe runs deepest: the lowest address.
        probes.borrow().iter().copied().min().expect("probe ran")
    };
    let shallow = sp(1);
    let deep = sp(levels);
    shallow.abs_diff(deep) / (levels - 1)
}

fn nested_substitution(n: usize) -> String {
    format!("echo {}probe{}", "$(".repeat(n), ")".repeat(n))
}

fn nested_functions(n: usize) -> String {
    // f1 calls f2 ... fn calls probe.
    let mut defs = String::new();
    for i in 1..n {
        defs.push_str(&format!("function f{i}() {{ f{} }}; ", i + 1));
    }
    defs.push_str(&format!("function f{n}() {{ probe }}; f1"));
    defs
}

fn nested_ifs(n: usize) -> String {
    format!("{}probe{}", "if true; then ".repeat(n), "; fi".repeat(n))
}

#[test]
fn nesting_fits_the_stack_budget() {
    let levels = 20;
    let costs = [
        ("$(...)", bytes_per_level(nested_substitution, levels)),
        ("function", bytes_per_level(nested_functions, levels)),
        ("if", bytes_per_level(nested_ifs, levels)),
    ];
    for (kind, cost) in costs {
        let total = cost * MAX_EXEC_DEPTH;
        println!(
            "{kind}: {cost} bytes/level, {} KiB at the {MAX_EXEC_DEPTH}-level limit",
            total / 1024
        );
        if !cfg!(debug_assertions) {
            assert!(
                total <= RELEASE_BUDGET,
                "{kind}: {cost} B/level x {MAX_EXEC_DEPTH} = {total} B over {RELEASE_BUDGET} B"
            );
        }
    }
}

/// Running into the nesting limit is an error, not a stack overflow, on a
/// 1 MiB stack (a Windows main thread) even in a debug build.
#[test]
fn nesting_limit_is_reached_cleanly() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut reg = CommandRegistry::new();
            oasis_terminal::register_builtins(&mut reg);
            let mut vfs = MemoryVfs::new();
            let deep_subst = format!("echo {}x{}", "$(echo ".repeat(200), ")".repeat(200));
            // The innermost substitutions fail (reported on stderr) and the
            // line still completes.
            let _ = reg.execute(&deep_subst, &mut env(&mut vfs));

            reg.execute("function r() { r }", &mut env(&mut vfs))
                .expect("define");
            let err = reg.execute("r", &mut env(&mut vfs)).unwrap_err();
            assert!(err.to_string().contains("recursion depth"), "{err}");

            // Blocks share the limit with substitutions and functions.
            let ifs = nested_ifs(MAX_EXEC_DEPTH + 8).replace("probe", "echo deep");
            let out = reg.execute(&ifs, &mut env(&mut vfs)).expect("runs");
            let text = format!("{out:?}");
            assert!(text.contains("maximum nesting depth"), "{text}");
            assert!(!text.contains("deep\""), "{text}");
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");
}
