#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Feed arbitrary command strings into the terminal interpreter.
    // Catches panics from malformed commands, pathological glob patterns,
    // variable expansion edge cases, pipe chains, etc.
    if let Ok(input) = std::str::from_utf8(data) {
        // Limit input size to prevent excessive runtime.
        if input.len() > 4096 {
            return;
        }

        let mut vfs = oasis_vfs::MemoryVfs::new();
        // Seed with a few files for commands to operate on.
        let _ = vfs.write("/test.txt", b"hello world");
        let _ = vfs.mkdir("/dir");

        let mut registry = oasis_terminal::CommandRegistry::new();
        oasis_terminal::register_builtins(&mut registry);

        let mut env = oasis_terminal::Environment::new(Box::new(vfs));
        let _ = registry.execute(input, &mut env);
    }
});
