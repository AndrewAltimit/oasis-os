//! Reading the shell-published state (`/system/state/*`) back into the
//! Settings UI.

use super::*;

/// Last raw values read from the shell-published preference state paths.
///
/// Preference controls (volume, font scale, reduced motion) update the UI
/// optimistically and post a request; the shell applies it a frame later.
/// Syncing only when a state file *changes* keeps a stale published value
/// from snapping the control back in the meantime.
#[derive(Debug, Default)]
pub(super) struct SeenState {
    pub(super) volume: Option<String>,
    pub(super) locale: Option<String>,
    pub(super) font_scale: Option<String>,
    pub(super) reduced_motion: Option<String>,
}

impl SettingsApp {
    /// Re-read the shell-published state from VFS. Called on input and on
    /// every tick so the UI reflects changes applied by the shell after we
    /// posted a request. Returns `true` when anything changed.
    pub(super) fn sync_from_vfs(&mut self, vfs: &dyn Vfs) -> bool {
        let mut changed = false;

        if let Some(skin) = read_utf8(vfs, SKIN_STATE_PATH)
            && skin != self.current_skin
        {
            self.current_skin = skin;
            if self.category == Category::Appearance {
                // The shell swapped skins under us -- reload the palette so
                // the editor reflects the actually-running theme (unless the
                // swap was our own preview, which keeps the same skin name
                // and therefore doesn't reach this branch).
                self.ensure_appearance_palette();
            }
            if self.category == Category::Display {
                let names = builtin_names();
                if let Some(idx) = names.iter().position(|n| *n == self.current_skin) {
                    self.item_cursor = idx;
                } else {
                    // Shell published a skin not in the builtin list (e.g.
                    // external TOML). Keep the cursor in bounds so navigation
                    // and `handle_confirm` stay safe even if the builtin list
                    // shrinks between syncs.
                    let max = names.len().saturating_sub(1);
                    if self.item_cursor > max {
                        self.item_cursor = max;
                    }
                }
            }
            changed = true;
        }

        if let Some((w, h)) =
            read_utf8(vfs, RESOLUTION_STATE_PATH).and_then(|s| parse_resolution(&s))
            && (w != self.width || h != self.height)
        {
            self.width = w;
            self.height = h;
            if self.category == Category::Resolution {
                if let Some(idx) = self.current_resolution_index() {
                    self.item_cursor = idx;
                } else {
                    let max = RESOLUTION_PRESETS.len().saturating_sub(1);
                    if self.item_cursor > max {
                        self.item_cursor = max;
                    }
                }
            }
            changed = true;
        }

        changed |= self.sync_prefs(vfs);

        if changed {
            self.refresh_lines();
        }
        changed
    }

    /// Adopt shell-published preference values that changed since the last
    /// sync (see [`SeenState`]). Returns `true` when anything changed.
    pub(super) fn sync_prefs(&mut self, vfs: &dyn Vfs) -> bool {
        let mut changed = false;
        if let Some(v) = read_changed(vfs, VOLUME_STATE_PATH, &mut self.seen.volume)
            .and_then(|s| s.parse::<u32>().ok())
        {
            changed |= v.min(100) != self.volume;
            self.volume = v.min(100);
        }
        if let Some(l) = read_changed(vfs, LOCALE_STATE_PATH, &mut self.seen.locale)
            .and_then(|s| Locale::from_code(&s))
        {
            changed |= l != self.locale;
            self.locale = l;
        }
        if let Some(f) = read_changed(vfs, FONT_SCALE_STATE_PATH, &mut self.seen.font_scale)
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|f| f.is_finite())
            .map(|f| {
                let (min, max) = font_scale_range();
                f.clamp(min, max)
            })
        {
            changed |= (f - self.font_scale).abs() > f32::EPSILON;
            self.font_scale = f;
        }
        if let Some(on) = read_changed(
            vfs,
            REDUCED_MOTION_STATE_PATH,
            &mut self.seen.reduced_motion,
        )
        .map(|s| parse_bool(&s))
        {
            changed |= on != self.reduced_motion;
            self.reduced_motion = on;
        }
        changed
    }
}

/// Read a VFS file as a trimmed UTF-8 string, returning `None` if missing or
/// not valid UTF-8.
pub(super) fn read_utf8(vfs: &dyn Vfs, path: &str) -> Option<String> {
    let data = vfs.read(path).ok()?;
    let s = std::str::from_utf8(&data).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Read a state file and return its value only if it differs from the last
/// value seen (updating `seen`).
pub(super) fn read_changed(vfs: &dyn Vfs, path: &str, seen: &mut Option<String>) -> Option<String> {
    let value = read_utf8(vfs, path)?;
    if seen.as_deref() == Some(value.as_str()) {
        return None;
    }
    *seen = Some(value.clone());
    Some(value)
}

/// Parse a boolean state payload (`1`/`true`/`on`).
pub(super) fn parse_bool(s: &str) -> bool {
    matches!(s.trim(), "1" | "true" | "on" | "yes")
}
