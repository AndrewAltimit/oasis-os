// Build script for oasis-skin: generates the built-in skin loaders from the
// TOML skin directories in `../../skins/`. Those directories are the single
// source of truth for every built-in skin.
//
// For each skin directory that contains `skin.toml`, `layout.toml`,
// `features.toml`, `theme.toml`, and `strings.toml`, this generates a Rust
// function `gen_{name}_skin() -> Result<Skin>` that embeds the files with
// `include_str!` and parses them with `Skin::from_sources()` -- the same
// parser `Skin::from_directory` uses -- including `corrupted.toml` when the
// directory has one. Files in the skin's
// `assets/` subdirectory are embedded with `include_bytes!`: PNG images via
// `Skin::add_asset_png`, TTF/OTF fonts via `Skin::add_asset_font`, WAV sounds
// via `Skin::add_asset_wav`.
//
// It also emits `GENERATED_SKIN_NAMES` (the built-in list: `classic` first,
// since it is the default and fallback skin, then the rest alphabetically)
// and the `load_generated_skin(name)` dispatch.

use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

/// The default skin; always listed first in `GENERATED_SKIN_NAMES`.
const DEFAULT_SKIN: &str = "classic";

struct SkinDir {
    dir_name: String,
    fn_name: String,
    corrupted: bool,
    images: Vec<String>,
    fonts: Vec<String>,
    sounds: Vec<String>,
}

impl SkinDir {
    fn has_assets(&self) -> bool {
        !(self.images.is_empty() && self.fonts.is_empty() && self.sounds.is_empty())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let skins_dir = Path::new(&manifest_dir).join("../../skins");
    let out_dir = env::var("OUT_DIR")?;
    let out_path = Path::new(&out_dir).join("builtin_generated.rs");

    // Re-run if any skin directory is added or removed.
    println!("cargo::rerun-if-changed={}", skins_dir.display());

    let mut skins: Vec<SkinDir> = Vec::new();

    if let Ok(entries) = fs::read_dir(&skins_dir) {
        let mut dirs: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        dirs.sort_by_key(|e| e.file_name());

        for entry in &dirs {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = entry.file_name().to_string_lossy().to_string();
            // Settings saves user skins as `skins/custom-<base>/` (relative
            // to the working directory, so into the repository when the
            // shell runs from it): those are not built-ins.
            if dir_name.starts_with("custom-") {
                continue;
            }

            let required = [
                "skin.toml",
                "layout.toml",
                "features.toml",
                "theme.toml",
                "strings.toml",
            ];
            // Register every file (present or not) for change detection.
            for f in required.iter().chain(&["corrupted.toml"]) {
                println!("cargo::rerun-if-changed={}", path.join(f).display());
            }
            if !required.iter().all(|f| path.join(f).exists()) {
                continue;
            }
            let corrupted = path.join("corrupted.toml").exists();

            // Collect assets from the skin's assets/ subdirectory.
            let assets_dir = path.join("assets");
            println!("cargo::rerun-if-changed={}", assets_dir.display());
            let mut images = Vec::new();
            let mut fonts = Vec::new();
            let mut sounds = Vec::new();
            if let Ok(asset_entries) = fs::read_dir(&assets_dir) {
                for ae in asset_entries.filter_map(|e| e.ok()) {
                    let ap = ae.path();
                    let ext_is = |wanted: &str| {
                        ap.extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case(wanted))
                    };
                    let bucket = if ext_is("png") {
                        &mut images
                    } else if ext_is("ttf") || ext_is("otf") {
                        &mut fonts
                    } else if ext_is("wav") {
                        &mut sounds
                    } else {
                        continue;
                    };
                    println!("cargo::rerun-if-changed={}", ap.display());
                    bucket.push(ae.file_name().to_string_lossy().to_string());
                }
                images.sort();
                fonts.sort();
                sounds.sort();
            }

            // Directory name to a valid Rust identifier ("retro-cga" -> "retro_cga").
            let fn_name = dir_name.replace('-', "_");
            skins.push(SkinDir {
                dir_name,
                fn_name,
                corrupted,
                images,
                fonts,
                sounds,
            });
        }
    }

    // Stable order: the default skin first, then alphabetical (already sorted).
    if let Some(pos) = skins.iter().position(|s| s.dir_name == DEFAULT_SKIN) {
        let default = skins.remove(pos);
        skins.insert(0, default);
    }

    let mut out = fs::File::create(&out_path)?;

    writeln!(
        out,
        "// Auto-generated skin loader functions from `skins/` TOML directories."
    )?;
    writeln!(out, "// Generated by build.rs -- do not edit manually.")?;
    writeln!(out)?;
    writeln!(out, "use oasis_types::error::Result;")?;
    writeln!(out)?;
    writeln!(out, "use crate::loader::{{Skin, SkinSources}};")?;
    writeln!(out)?;

    let file = |dir: &str, name: &str| {
        format!(
            "include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \
             \"/../../skins/{dir}/{name}\"))"
        )
    };
    let bytes = |dir: &str, name: &str| {
        format!(
            "include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \
             \"/../../skins/{dir}/assets/{name}\"))"
        )
    };

    for skin in &skins {
        let d = skin.dir_name.as_str();
        let with_assets = skin.has_assets();
        writeln!(
            out,
            "/// Load the `{d}` skin (generated from `skins/{d}/`)."
        )?;
        writeln!(out, "fn gen_{}_skin() -> Result<Skin> {{", skin.fn_name)?;
        if with_assets {
            writeln!(out, "    let mut skin = Skin::from_sources(&SkinSources {{")?;
        } else {
            // No assets: the parse result is the tail expression.
            writeln!(out, "    Skin::from_sources(&SkinSources {{")?;
        }
        for (field, name) in [
            ("manifest", "skin.toml"),
            ("layout", "layout.toml"),
            ("features", "features.toml"),
            ("theme", "theme.toml"),
            ("strings", "strings.toml"),
        ] {
            writeln!(out, "        {field}: {},", file(d, name))?;
        }
        if skin.corrupted {
            writeln!(
                out,
                "        corrupted: Some({}),",
                file(d, "corrupted.toml")
            )?;
        } else {
            writeln!(out, "        corrupted: None,")?;
        }
        writeln!(out, "    }}){}", if with_assets { "?;" } else { "" })?;
        for img in &skin.images {
            writeln!(
                out,
                "    skin.add_asset_png(\"assets/{img}\", {})?;",
                bytes(d, img)
            )?;
        }
        for font in &skin.fonts {
            writeln!(
                out,
                "    skin.add_asset_font(\"assets/{font}\", {}.to_vec());",
                bytes(d, font)
            )?;
        }
        for snd in &skin.sounds {
            writeln!(
                out,
                "    skin.add_asset_wav(\"assets/{snd}\", {})?;",
                bytes(d, snd)
            )?;
        }
        if with_assets {
            writeln!(out, "    Ok(skin)")?;
        }
        writeln!(out, "}}")?;
        writeln!(out)?;
    }

    writeln!(
        out,
        "/// Names of the built-in skins: `{DEFAULT_SKIN}` first, then alphabetical."
    )?;
    writeln!(out, "pub const GENERATED_SKIN_NAMES: &[&str] = &[")?;
    for skin in &skins {
        writeln!(out, "    \"{}\",", skin.dir_name)?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;

    writeln!(
        out,
        "/// Load a generated skin by name (`None` if there is no such skin)."
    )?;
    writeln!(
        out,
        "pub fn load_generated_skin(name: &str) -> Option<Result<Skin>> {{"
    )?;
    writeln!(out, "    match name {{")?;
    for skin in &skins {
        writeln!(
            out,
            "        \"{}\" => Some(gen_{}_skin()),",
            skin.dir_name, skin.fn_name
        )?;
    }
    writeln!(out, "        _ => None,")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;

    Ok(())
}
