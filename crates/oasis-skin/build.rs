// Build script for oasis-skin: generates the built-in skin loaders from the
// TOML skin directories in `../../skins/`. Those directories are the single
// source of truth for every built-in skin.
//
// For each skin directory that contains `skin.toml`, `layout.toml`,
// `features.toml`, `theme.toml`, and `strings.toml`, this generates a Rust
// function `gen_skin_<n>() -> Result<Skin>` that embeds the files with
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
//
// Robustness: loader functions are numbered (not derived from the directory
// name, which may not be a valid or unique Rust identifier), every embedded
// path / name is written as an escaped string literal, directories that are
// almost skins (a `skin.toml` but a missing file) or have non-UTF-8 names
// produce a cargo warning instead of silently vanishing, and the output file
// is only rewritten when its content changes.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// The default skin; always listed first in `GENERATED_SKIN_NAMES`.
const DEFAULT_SKIN: &str = "classic";

/// Files every built-in skin directory must have.
const REQUIRED: [&str; 5] = [
    "skin.toml",
    "layout.toml",
    "features.toml",
    "theme.toml",
    "strings.toml",
];

struct SkinDir {
    dir_name: String,
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

    let mut skins = collect_skins(&skins_dir);

    // Stable order: the default skin first, then alphabetical (already sorted).
    match skins.iter().position(|s| s.dir_name == DEFAULT_SKIN) {
        Some(pos) => {
            let default = skins.remove(pos);
            skins.insert(0, default);
        },
        None => println!(
            "cargo::warning=no `{DEFAULT_SKIN}` skin under {}: the default/fallback \
             skin will fail to load",
            skins_dir.display()
        ),
    }

    let code = generate(&skins)?;
    // Leave an identical file untouched so its mtime does not trigger
    // needless recompiles.
    if fs::read_to_string(&out_path).ok().as_deref() != Some(code.as_str()) {
        fs::write(&out_path, code)?;
    }
    Ok(())
}

/// Scan `skins_dir` for complete skin directories, sorted by name.
fn collect_skins(skins_dir: &Path) -> Vec<SkinDir> {
    let Ok(entries) = fs::read_dir(skins_dir) else {
        println!(
            "cargo::warning=skin directory {} not readable: no built-in skins",
            skins_dir.display()
        );
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    dirs.sort_by_key(|e| e.file_name());

    let mut skins = Vec::new();
    for entry in &dirs {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(dir_name) = entry.file_name().to_str().map(str::to_owned) else {
            println!(
                "cargo::warning=skipping skin directory with a non-UTF-8 name: {}",
                path.display()
            );
            continue;
        };
        // Settings saves user skins as `skins/custom-<base>/` (relative
        // to the working directory, so into the repository when the
        // shell runs from it): those are not built-ins.
        if dir_name.starts_with("custom-") {
            continue;
        }

        // Register every file (present or not) for change detection.
        for f in REQUIRED.iter().chain(&["corrupted.toml"]) {
            println!("cargo::rerun-if-changed={}", path.join(f).display());
        }
        let missing: Vec<&str> = REQUIRED
            .iter()
            .copied()
            .filter(|f| !path.join(f).is_file())
            .collect();
        if !missing.is_empty() {
            // A directory with a manifest is meant to be a skin: say why it
            // is not built in rather than dropping it without a word.
            if path.join("skin.toml").is_file() {
                println!(
                    "cargo::warning=skins/{dir_name} is not built in: missing {}",
                    missing.join(", ")
                );
            }
            continue;
        }
        let corrupted = path.join("corrupted.toml").is_file();

        // Collect assets from the skin's assets/ subdirectory.
        let assets_dir = path.join("assets");
        println!("cargo::rerun-if-changed={}", assets_dir.display());
        let (mut images, mut fonts, mut sounds) = (Vec::new(), Vec::new(), Vec::new());
        if let Ok(asset_entries) = fs::read_dir(&assets_dir) {
            for ae in asset_entries.filter_map(|e| e.ok()) {
                let ap = ae.path();
                if !ap.is_file() {
                    continue;
                }
                let ext = ap
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase);
                let bucket = match ext.as_deref() {
                    Some("png") => &mut images,
                    Some("ttf" | "otf") => &mut fonts,
                    Some("wav") => &mut sounds,
                    _ => continue,
                };
                let Some(name) = ae.file_name().to_str().map(str::to_owned) else {
                    println!(
                        "cargo::warning=skipping asset with a non-UTF-8 name: {}",
                        ap.display()
                    );
                    continue;
                };
                println!("cargo::rerun-if-changed={}", ap.display());
                bucket.push(name);
            }
            images.sort();
            fonts.sort();
            sounds.sort();
        }

        skins.push(SkinDir {
            dir_name,
            corrupted,
            images,
            fonts,
            sounds,
        });
    }
    skins
}

/// `include_str!` / `include_bytes!` of a path under `skins/`, with the
/// relative part written as an escaped string literal.
fn include(mac: &str, rel: &str) -> String {
    let suffix = format!("/../../skins/{rel}");
    format!("{mac}!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {suffix:?}))")
}

/// The generated Rust source.
fn generate(skins: &[SkinDir]) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
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

    for (i, skin) in skins.iter().enumerate() {
        let d = skin.dir_name.as_str();
        let with_assets = skin.has_assets();
        // `{:?}` keeps an odd directory name from breaking the doc comment.
        writeln!(out, "/// Load the {d:?} skin (generated from `skins/`).")?;
        writeln!(out, "fn gen_skin_{i}() -> Result<Skin> {{")?;
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
            writeln!(
                out,
                "        {field}: {},",
                include("include_str", &format!("{d}/{name}"))
            )?;
        }
        if skin.corrupted {
            writeln!(
                out,
                "        corrupted: Some({}),",
                include("include_str", &format!("{d}/corrupted.toml"))
            )?;
        } else {
            writeln!(out, "        corrupted: None,")?;
        }
        writeln!(out, "    }}){}", if with_assets { "?;" } else { "" })?;
        let asset = |name: &str| {
            (
                format!("{:?}", format!("assets/{name}")),
                include("include_bytes", &format!("{d}/assets/{name}")),
            )
        };
        for img in &skin.images {
            let (key, bytes) = asset(img);
            writeln!(out, "    skin.add_asset_png({key}, {bytes})?;")?;
        }
        for font in &skin.fonts {
            let (key, bytes) = asset(font);
            writeln!(out, "    skin.add_asset_font({key}, {bytes}.to_vec());")?;
        }
        for snd in &skin.sounds {
            let (key, bytes) = asset(snd);
            writeln!(out, "    skin.add_asset_wav({key}, {bytes})?;")?;
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
    for skin in skins {
        writeln!(out, "    {:?},", skin.dir_name)?;
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
    for (i, skin) in skins.iter().enumerate() {
        writeln!(out, "        {:?} => Some(gen_skin_{i}()),", skin.dir_name)?;
    }
    writeln!(out, "        _ => None,")?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")?;
    Ok(out)
}
