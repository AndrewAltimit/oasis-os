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
    ("altimit", 0xa5fefb05e07352aa),
    ("balatro", 0x7bdf1ad9bbc634a0),
    ("classic", 0x9f6bc1d5f14c3d3d),
    ("corrupted", 0xa06c163338056501),
    ("desktop", 0x10856f7fe87b9d35),
    ("gnome", 0x390d654575537ccc),
    ("highcontrast", 0xbd7ca4959a7ec556),
    ("macos", 0x82e938e340840589),
    ("modern", 0x3608c816635a29ed),
    ("paper", 0x2cff6b69dc956183),
    ("psix-hifi", 0x68548a7a9111db42),
    ("psix-tribute", 0x799c1f869e549b17),
    ("retro-cga", 0x22d654c4fc5a6c40),
    ("solarized", 0x62591788b0c9908c),
    ("vaporwave", 0x51f1f5fbee96a283),
    ("win95", 0xbb85b1c3ce4841e9),
    ("xp", 0x3fe6c4c5467ec01d),
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
pub(crate) fn canonical_dump(skin: &Skin) -> String {
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
