//! Terminal commands for skin management.

use oasis_skin::Skin;
use oasis_skin::builtin;
use oasis_types::error::Result;

use crate::{Command, CommandOutput, CommandRegistry, Environment};

/// Register skin-related commands.
pub fn register_skin_commands(reg: &mut CommandRegistry) {
    reg.register(Box::new(SkinCmd));
}

/// Terminal command for listing, showing, or switching UI skins.
struct SkinCmd;

impl Command for SkinCmd {
    fn name(&self) -> &str {
        "skin"
    }

    fn description(&self) -> &str {
        "List, show, or switch skins"
    }

    fn usage(&self) -> &str {
        "skin [list|current|<name>]"
    }

    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        match args.first().copied() {
            None | Some("list") => {
                let mut lines = String::from("Built-in skins:\n");
                for name in builtin::builtin_names() {
                    lines.push_str(&format!("  {name}\n"));
                }

                let discovered = Skin::discover_skins(std::path::Path::new("skins"));
                if !discovered.is_empty() {
                    lines.push_str("\nExternal skins:\n");
                    for (name, path) in &discovered {
                        lines.push_str(&format!("  {name}  ({})\n", path.display()));
                    }
                }

                Ok(CommandOutput::Text(lines))
            },
            Some("current") => Ok(CommandOutput::Text(
                "Use 'skin <name>' to switch skins.".to_string(),
            )),
            Some(name) => Ok(CommandOutput::SkinSwap {
                name: name.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        }
    }

    #[test]
    fn skin_list() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let out = cmd.execute(&["list"], &mut env).unwrap();
        match out {
            CommandOutput::Text(s) => {
                assert!(s.contains("terminal"));
                assert!(s.contains("modern"));
            },
            _ => panic!("expected Text"),
        }
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
        match out {
            CommandOutput::SkinSwap { name } => assert_eq!(name, "modern"),
            _ => panic!("expected SkinSwap"),
        }
    }
}
