//! Snapshot guard for every built-in skin.
//!
//! Each built-in is reduced to a canonical text dump: every parsed table
//! (manifest, features, theme, strings, layout, corrupted modifiers) goes
//! through `toml::Value`, whose tables are `BTreeMap`-backed, so the dump does
//! not depend on `HashMap` iteration order. Embedded assets contribute their
//! path, size and a byte hash. The test compares an FNV-1a hash of each dump
//! against the table below, so any change to what a built-in skin *parses
//! to* -- not just to its TOML text -- is caught.
//!
//! When you change a skin under `skins/` on purpose, the failure message
//! prints the new table; paste it over `EXPECTED`. To inspect the dumps
//! themselves, set `OASIS_SKIN_SNAPSHOT_DIR=<dir>` and the test writes one
//! `<name>.txt` per skin there.

use super::{builtin_names, load_builtin};
use crate::loader::Skin;

/// `(skin name, FNV-1a 64 hash of its canonical dump)`, sorted by name.
const EXPECTED: &[(&str, u64)] = &[
    ("altimit", 0xd03fd5b7d4906bc3),
    ("balatro", 0xf44a0d9e1547b321),
    ("classic", 0x8b0d27d44af1c9d6),
    ("corrupted", 0x977071de551b9eec),
    ("desktop", 0x5cd6ce21999bbcc8),
    ("gnome", 0x10e8b2489edf4627),
    ("highcontrast", 0x89c58c9e6de1652f),
    ("macos", 0x9e3cd02fdc8d2e2a),
    ("modern", 0x565556275e1a093e),
    ("paper", 0x7139dface497d3c0),
    ("psix-hifi", 0xe11207bba30aa539),
    ("psix-tribute", 0x7b5aa4e2569cd4fa),
    ("retro-cga", 0xaf965950465c35af),
    ("solarized", 0xdb6998e0ea866b91),
    ("vaporwave", 0xb2e0258bc1240f8e),
    ("win95", 0x266e76e473c93dab),
    ("xp", 0x30ff2fe72e6ee580),
];

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn section<T: serde::Serialize>(out: &mut String, name: &str, value: &T) {
    let value = toml::Value::try_from(value).expect("skin section converts to toml::Value");
    let body = match value {
        toml::Value::Table(t) => toml::to_string(&t).expect("table serializes"),
        other => format!("{other}\n"),
    };
    out.push_str(&format!("### {name}\n{body}\n"));
}

/// Canonical, order-independent text form of a loaded skin.
pub(super) fn canonical_dump(skin: &Skin) -> String {
    let mut out = String::new();
    section(&mut out, "manifest", &skin.manifest);
    section(&mut out, "features", &skin.features);
    section(&mut out, "theme", &skin.theme);
    section(&mut out, "strings", &skin.strings);
    section(&mut out, "layout", &skin.layout);
    out.push_str(&format!(
        "### layout object count\n{}\n\n",
        skin.layout.objects.len()
    ));
    match skin.corrupted_modifiers {
        Some(ref m) => section(&mut out, "corrupted", m),
        None => out.push_str("### corrupted\nnone\n\n"),
    }
    out.push_str("### schema_warnings\n");
    for w in &skin.schema_warnings {
        out.push_str(&format!("{w}\n"));
    }
    out.push_str("\n### assets\n");
    let mut images: Vec<_> = skin.assets.iter().collect();
    images.sort_by(|a, b| a.0.cmp(b.0));
    for (path, asset) in images {
        out.push_str(&format!(
            "image {path} {}x{} {:016x}\n",
            asset.width,
            asset.height,
            fnv1a64(&asset.rgba)
        ));
    }
    let mut fonts: Vec<_> = skin.font_assets.iter().collect();
    fonts.sort_by(|a, b| a.0.cmp(b.0));
    for (path, bytes) in fonts {
        out.push_str(&format!(
            "font {path} {} {:016x}\n",
            bytes.len(),
            fnv1a64(bytes)
        ));
    }
    let mut sounds: Vec<_> = skin.sound_assets.iter().collect();
    sounds.sort_by(|a, b| a.0.cmp(b.0));
    for (path, bytes) in sounds {
        out.push_str(&format!(
            "sound {path} {} {:016x}\n",
            bytes.len(),
            fnv1a64(bytes)
        ));
    }
    out
}

#[test]
fn builtin_skin_snapshots_match() {
    let dump_dir = std::env::var_os("OASIS_SKIN_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    if let Some(ref dir) = dump_dir {
        std::fs::create_dir_all(dir).expect("create snapshot dir");
    }

    let mut names: Vec<&str> = builtin_names().to_vec();
    names.sort_unstable();

    let mut actual = Vec::new();
    for name in &names {
        let skin = load_builtin(name).unwrap_or_else(|e| panic!("{name} loads: {e}"));
        let dump = canonical_dump(&skin);
        if let Some(ref dir) = dump_dir {
            std::fs::write(dir.join(format!("{name}.txt")), &dump).expect("write snapshot");
        }
        actual.push((*name, fnv1a64(dump.as_bytes())));
    }

    if actual.as_slice() != EXPECTED {
        let table: String = actual
            .iter()
            .map(|(n, h)| format!("    (\"{n}\", 0x{h:016x}),\n"))
            .collect();
        panic!(
            "built-in skin snapshot changed. If the skin change is intended, \
             replace EXPECTED in src/builtin/snapshot_tests.rs with:\n{table}\
             (set OASIS_SKIN_SNAPSHOT_DIR to dump the canonical text and diff it)"
        );
    }
}
