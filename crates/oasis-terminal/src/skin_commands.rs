//! Terminal commands for skin management.

#[cfg(not(target_arch = "wasm32"))]
use oasis_skin::Skin;
use oasis_skin::builtin;
use oasis_skin::{SkinVariant, VARIANT_REQUEST_PREFIX, resolve_skin};

use crate::CommandOutput;
use crate::interpreter::resolve_path;

register_commands!(register_skin_commands, [SkinCmd]);

/// Resolve a skin for linting: built-in name, explicit path, or
/// `./skins/{name}/`. Unlike `resolve_skin`, does NOT fall back to classic —
/// a lint of a missing skin must be an error, not a lint of the fallback.
fn load_for_lint(target: &str) -> Option<oasis_types::error::Result<oasis_skin::Skin>> {
    if let Ok(skin) = builtin::load_builtin(target) {
        return Some(Ok(skin));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::path::Path::new(target);
        if path.join("skin.toml").is_file() {
            return Some(Skin::from_directory(path));
        }
        let skins_dir = std::path::Path::new("skins").join(target);
        if skins_dir.join("skin.toml").is_file() {
            return Some(Skin::from_directory(&skins_dir));
        }
    }
    None
}

/// Run `Skin::validate` on a named skin and format the report.
fn lint_skin(target: &str) -> String {
    match load_for_lint(target) {
        None => format!("skin '{target}' not found (built-in name, path, or skins/ subdir)"),
        Some(Err(e)) => format!("skin '{target}' failed to parse: {e}"),
        Some(Ok(skin)) => {
            let warnings = skin.validate();
            if warnings.is_empty() {
                format!(
                    "{target}: clean ({} layout objects, {} assets)",
                    skin.layout.objects.len(),
                    skin.assets.len()
                )
            } else {
                let mut out = format!("{target}: {} warning(s)\n", warnings.len());
                for w in &warnings {
                    out.push_str(&format!("  {w}\n"));
                }
                out
            }
        },
    }
}

// Terminal command for listing, showing, linting, switching, exporting, or
// deriving variants of UI skins.
define_command!(
    SkinCmd,
    "skin",
    "List, show, lint, switch, export, or derive variants of skins",
    "skin [list|current|lint <name>|export <name> [file]|variant <dark|light|high-contrast>|<name>]",
    "ui",
    |args, env| {
        match args.first().copied() {
            Some("lint") => {
                let Some(target) = args.get(1).copied() else {
                    return Ok(CommandOutput::Text(
                        "usage: skin lint <name|path>".to_string(),
                    ));
                };
                Ok(CommandOutput::Text(lint_skin(target)))
            },
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
            Some("export") => {
                let Some(name) = args.get(1).copied() else {
                    return Err(oasis_types::error::OasisError::Command(
                        "usage: skin export <name> [file]".into(),
                    ));
                };
                let skin = resolve_skin(name)?;
                let toml_doc = skin.to_toml_string()?;
                let dest = args
                    .get(2)
                    .map(|f| resolve_path(&env.cwd, f))
                    .unwrap_or_else(|| resolve_path(&env.cwd, &format!("{name}.skin.toml")));
                env.vfs.write(&dest, toml_doc.as_bytes()).map_err(|e| {
                    oasis_types::error::OasisError::Command(format!("write {dest}: {e}").into())
                })?;
                Ok(CommandOutput::Text(format!(
                    "Exported skin '{}' to {dest} ({} bytes)",
                    skin.manifest.name,
                    toml_doc.len()
                )))
            },
            Some("variant") => {
                let Some(v) = args.get(1).copied() else {
                    return Err(oasis_types::error::OasisError::Command(
                        "usage: skin variant <dark|light|high-contrast>".into(),
                    ));
                };
                let Some(variant) = SkinVariant::from_name(v) else {
                    return Err(oasis_types::error::OasisError::Command(
                        format!("unknown variant '{v}' (dark|light|high-contrast)").into(),
                    ));
                };
                // The app layer resolves this against the currently active
                // skin (see `oasis_skin::resolve_skin_request`).
                Ok(CommandOutput::skin_swap(format!(
                    "{VARIANT_REQUEST_PREFIX}{}",
                    variant.name()
                )))
            },
            Some(name) => Ok(CommandOutput::skin_swap(name.to_string())),
        }
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{Command, CommandSignal, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

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
    fn skin_lint_builtin_clean() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["lint", "classic"], &mut env).unwrap());
        assert!(s.contains("clean"), "unexpected lint output: {s}");
    }

    #[test]
    fn skin_lint_unknown_name() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["lint", "no-such-skin"], &mut env).unwrap());
        assert!(s.contains("not found"), "unexpected lint output: {s}");
    }

    #[test]
    fn skin_lint_no_target_shows_usage() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["lint"], &mut env).unwrap());
        assert!(s.contains("usage"));
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

    #[test]
    fn skin_export_writes_reloadable_document() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["export", "classic"], &mut env).unwrap());
        assert!(s.contains("Exported skin 'classic'"));
        let data = vfs.read("/classic.skin.toml").expect("export file exists");
        let doc = std::str::from_utf8(&data).expect("utf8");
        let reloaded = oasis_skin::Skin::from_toml_string(doc).expect("re-parses");
        assert_eq!(reloaded.manifest.name, "classic");
    }

    #[test]
    fn skin_export_custom_destination() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(
            cmd.execute(&["export", "paper", "/paper-out.toml"], &mut env)
                .unwrap()
        );
        assert!(s.contains("/paper-out.toml"));
        assert!(vfs.read("/paper-out.toml").is_ok());
    }

    #[test]
    fn skin_export_missing_name_errors() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        assert!(cmd.execute(&["export"], &mut env).is_err());
    }

    #[test]
    fn skin_variant_emits_variant_request() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let out = cmd.execute(&["variant", "dark"], &mut env).unwrap();
        let CommandOutput::Signal(CommandSignal::SkinSwap { name }) = out else {
            panic!("expected SkinSwap, got {out:?}");
        };
        assert_eq!(name, "@variant:dark");
    }

    #[test]
    fn skin_variant_rejects_unknown() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        assert!(cmd.execute(&["variant", "sepia"], &mut env).is_err());
        assert!(cmd.execute(&["variant"], &mut env).is_err());
    }
}
