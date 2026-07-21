//! Terminal commands for skin management.

#[cfg(not(target_arch = "wasm32"))]
use oasis_skin::Skin;
use oasis_skin::builtin;
use oasis_skin::theme::contrast_ratio;
use oasis_skin::{ActiveTheme, SkinVariant, VARIANT_REQUEST_PREFIX, resolve_skin};
use oasis_types::backend::Color;

use crate::CommandOutput;
use crate::interpreter::resolve_path;

/// Format a color's RGB channels as `#RRGGBB` (alpha dropped).
fn fmt_hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

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

/// Render a plain-text "contact sheet" for a resolved skin: the nine resolved
/// base colors, a WCAG AA contrast report for the key foreground/background
/// pairs, and a dump of the derived bar / icon / start-menu / app-screen
/// tokens plus the ANSI palette rows.
///
/// Output is plain ASCII (no color codes, no heavy deps) so it renders the
/// same on desktop, PSP, and WASM. Skin resolution matches `skin lint`: a
/// built-in name, a directory path, or a `skins/<name>/` subdir — with no
/// silent fallback to `classic`, so inspecting a missing skin is an error.
fn inspect_skin(target: &str) -> String {
    let skin = match load_for_lint(target) {
        None => {
            return format!("skin '{target}' not found (built-in name, path, or skins/ subdir)");
        },
        Some(Err(e)) => return format!("skin '{target}' failed to parse: {e}"),
        Some(Ok(skin)) => skin,
    };
    let theme = &skin.theme;
    let mut out = String::new();

    out.push_str(&format!(
        "Skin: {}  ({}x{})\n\n",
        skin.manifest.name, skin.manifest.screen_width, skin.manifest.screen_height,
    ));

    // -- Base colors (the nine roles a skin derives everything from) --
    let status_bar =
        oasis_skin::parse_hex_color(&theme.status_bar).unwrap_or(Color::rgb(40, 60, 90));
    let base: [(&str, Color); 9] = [
        ("background", theme.background_color()),
        ("primary", theme.primary_color()),
        ("secondary", theme.secondary_color()),
        ("text", theme.text_color()),
        ("dim_text", theme.dim_text_color()),
        ("status_bar", status_bar),
        ("prompt", theme.prompt_color()),
        ("output", theme.output_color()),
        ("error", theme.error_color()),
    ];
    out.push_str("Base colors:\n");
    for (name, c) in base {
        out.push_str(&format!("  {name:<11} {}\n", fmt_hex(c)));
    }

    // -- WCAG AA contrast report (same pairs the lint checks, but every pair
    // is listed with its ratio and verdict, not just the failing ones). --
    let bg = theme.background_color();
    let pairs: [(&str, Color, f64); 5] = [
        ("text on background", theme.text_color(), 4.5),
        ("dim_text on background", theme.dim_text_color(), 3.0),
        ("prompt on background", theme.prompt_color(), 3.0),
        ("output on background", theme.output_color(), 3.0),
        ("error on background", theme.error_color(), 3.0),
    ];
    out.push_str("\nWCAG contrast (AA):\n");
    for (label, fg, required) in pairs {
        let ratio = contrast_ratio(fg, bg);
        let verdict = if ratio >= required { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "  {label:<24} {ratio:5.2}:1  {verdict} (>= {required:.1})\n"
        ));
    }

    // -- Derived token colors, grouped by the sub-theme that owns them. --
    let at = ActiveTheme::from_skin(theme);
    out.push_str("\nTokens:\n");
    out.push_str(&format!(
        "  bar    statusbar_bg {}  bottom_bg {}  clock {}  tab_active {}\n",
        fmt_hex(at.bar.statusbar_bg),
        fmt_hex(at.bar.bg),
        fmt_hex(at.bar.clock_color),
        fmt_hex(at.bar.tab_active_fill),
    ));
    out.push_str(&format!(
        "  icon   body {}  outline {}  label {}  cursor {}\n",
        fmt_hex(at.icon.body_color),
        fmt_hex(at.icon.outline_color),
        fmt_hex(at.icon.label_color),
        fmt_hex(at.icon.cursor_color),
    ));
    out.push_str(&format!(
        "  menu   panel_bg {}  item_text {}  highlight {}  button_bg {}\n",
        fmt_hex(at.menu.panel_bg),
        fmt_hex(at.menu.item_text),
        fmt_hex(at.menu.highlight_color),
        fmt_hex(at.menu.button_bg),
    ));
    out.push_str(&format!(
        "  app    bg {}  text {}  title_bar_bg {}  selected_bg {}\n",
        fmt_hex(at.app.bg),
        fmt_hex(at.app.text),
        fmt_hex(at.app.title_bar_bg),
        fmt_hex(at.app.selected_bg),
    ));

    // ANSI palette: slots 0-7 (normal) and 8-15 (bright) on two rows.
    out.push_str("  ansi  ");
    for i in 0..8 {
        out.push(' ');
        out.push_str(&fmt_hex(at.ansi.color(i)));
    }
    out.push_str("\n  bright");
    for i in 8..16 {
        out.push(' ');
        out.push_str(&fmt_hex(at.ansi.color(i)));
    }
    out.push('\n');

    out
}

// Terminal command for listing, showing, linting, switching, exporting, or
// deriving variants of UI skins.
define_command!(
    SkinCmd,
    "skin",
    "List, show, lint, inspect, switch, export, or derive variants of skins",
    "skin [list|current|lint <name>|inspect <name>|export <name> [file]|\
     variant <dark|light|high-contrast>|<name>]",
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
            Some("inspect") => {
                let Some(target) = args.get(1).copied() else {
                    return Ok(CommandOutput::Text(
                        "usage: skin inspect <name|path>".to_string(),
                    ));
                };
                Ok(CommandOutput::Text(inspect_skin(target)))
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
    fn skin_inspect_reports_all_sections() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["inspect", "classic"], &mut env).unwrap());
        // Header names the skin and its resolution.
        assert!(s.contains("Skin: classic"), "missing header: {s}");
        // Each contact-sheet section is present.
        assert!(s.contains("Base colors:"), "missing base colors: {s}");
        assert!(s.contains("WCAG contrast (AA):"), "missing contrast: {s}");
        assert!(s.contains("Tokens:"), "missing tokens: {s}");
        // Base-color roles and token groups are labelled.
        assert!(s.contains("background"), "missing base role: {s}");
        assert!(s.contains("status_bar"), "missing status_bar role: {s}");
        assert!(
            s.contains("text on background"),
            "missing contrast pair: {s}"
        );
        assert!(s.contains("bar    "), "missing bar tokens: {s}");
        assert!(s.contains("ansi"), "missing ansi row: {s}");
        assert!(s.contains("bright"), "missing bright ansi row: {s}");
        // Contrast verdicts render as PASS/FAIL and colors as #RRGGBB.
        assert!(
            s.contains("PASS") || s.contains("FAIL"),
            "missing verdict: {s}"
        );
        assert!(s.contains('#'), "missing hex colors: {s}");
    }

    #[test]
    fn skin_inspect_unknown_name() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["inspect", "no-such-skin"], &mut env).unwrap());
        assert!(s.contains("not found"), "unexpected inspect output: {s}");
    }

    #[test]
    fn skin_inspect_no_target_shows_usage() {
        let cmd = SkinCmd;
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let s = assert_text!(cmd.execute(&["inspect"], &mut env).unwrap());
        assert!(s.contains("usage"), "unexpected inspect output: {s}");
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
