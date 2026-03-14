//! Shared helpers for SDI view modules.

use oasis_core::sdi::SdiRegistry;

/// Format an SDI object name into a reusable buffer, returning `&str`.
/// The buffer is cleared and rewritten on each call, so the returned
/// reference is only valid until the next `sdi_key!` call on the same buffer.
macro_rules! sdi_key {
    ($buf:expr, $($arg:tt)*) => {{
        $buf.clear();
        ::core::fmt::Write::write_fmt(&mut $buf, format_args!($($arg)*)).unwrap();
        $buf.as_str()
    }};
}
pub(crate) use sdi_key;

/// Ensure an object exists in the registry, creating it if necessary.
pub(crate) fn ensure(sdi: &mut SdiRegistry, name: &str) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
}

/// Only update `obj.text` when the value actually changed, avoiding a heap
/// allocation + drop on every frame for static content.
pub(crate) fn set_text(slot: &mut Option<String>, value: &str) {
    match slot {
        Some(existing) if existing == value => {},
        _ => *slot = Some(value.to_owned()),
    }
}

/// Hide all SDI objects whose name starts with `prefix`.
pub(crate) fn hide_prefixed(sdi: &mut SdiRegistry, prefix: &str) {
    let names: Vec<String> = sdi
        .names()
        .filter(|n| n.starts_with(prefix))
        .map(|n| n.to_string())
        .collect();
    for name in &names {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
}

/// All view prefixes.
const VIEW_PREFIXES: &[&str] = &["radio_", "tv_", "photo_", "browser_", "music_", "fm_"];

/// Hide all view objects (called on view transition).
pub(crate) fn hide_all(sdi: &mut SdiRegistry) {
    for prefix in VIEW_PREFIXES {
        hide_prefixed(sdi, prefix);
    }
}
