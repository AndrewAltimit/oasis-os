//! Terminal commands for skin management.

#[cfg(not(target_arch = "wasm32"))]
use oasis_skin::Skin;
use oasis_skin::builtin;

use crate::CommandOutput;

register_commands!(register_skin_commands, [SkinCmd]);

// Terminal command for listing, showing, or switching UI skins.
define_command!(
    SkinCmd,
    "skin",
    "List, show, or switch skins",
    "skin [list|current|<name>]",
    "ui",
    |args, _env| {
        match args.first().copied() {
            None | Some("list") => {
                let mut lines = String::from("Built-in skins:\n");
                for name in builtin::builtin_names() {
                    lines.push_str(&format!("  {name}\n"));
                }

                #[cfg(not(target_arch = "wasm32"))]
                {
                    let discovered = Skin::discover_skins(std::path::Path::new("skins"));
                    if !discovered.is_empty() {
                        lines.push_str("\nExternal skins:\n");
                        for (name, path) in &discovered {
                            lines.push_str(&format!("  {name}  ({})\n", path.display()));
                        }
                    }
                }

                Ok(CommandOutput::Text(lines))
            },
            Some("current") => Ok(CommandOutput::Text(
                "Use 'skin <name>' to switch skins.".to_string(),
            )),
            Some(name) => Ok(CommandOutput::skin_swap(name.to_string())),
        }
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{Command, CommandSignal, Environment};
    use oasis_vfs::MemoryVfs;

    fn make_env(vfs: &mut MemoryVfs) -> Environment<'_> {
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

    #[test]
    fn skin_list() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["list"], &mut env).unwrap());
        assert!(s.contains("classic"));
        assert!(s.contains("modern"));
    }

    #[test]
    fn skin_no_args_is_list() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let out = cmd.execute(&[], &mut env).unwrap();
        assert!(matches!(out, CommandOutput::Text(_)));
    }

    #[test]
    fn skin_swap_emits_signal() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let out = cmd.execute(&["modern"], &mut env).unwrap();
        let CommandOutput::Signal(CommandSignal::SkinSwap { name }) = out else {
            panic!("expected SkinSwap, got {out:?}");
        };
        assert_eq!(name, "modern");
    }
}
