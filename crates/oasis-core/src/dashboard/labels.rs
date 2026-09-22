//! Label rendering for dashboard icons.
//!
//! Provides word-wrapping and centered label drawing beneath icons,
//! including optional drop-shadow support.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::sdi::SdiRegistry;

use super::IconNames;

/// Horizontal gutter (px) kept free on each side of a label inside its
/// cell, so labels of neighbouring cells never touch.
pub(crate) const LABEL_GUTTER: u32 = 2;

/// Maximum number of label lines under an icon.
pub(crate) const LABEL_MAX_LINES: usize = 2;

/// Cache of fitted label lines, keyed by app title. The stored key is the
/// `(max_width_px, font_size)` the lines were fitted at, so a font or
/// cell-width change lazily re-fits instead of serving stale lines.
/// Titles are static frame-to-frame; without this every icon re-measures
/// and re-allocates its label lines 60 times a second.
pub(crate) type LabelWrapCache = RefCell<HashMap<String, ((u32, u16), Vec<String>)>>;

/// Pixel budget for one label line in a cell `cell_w` pixels wide.
pub(crate) fn label_max_width(cell_w: u32) -> u32 {
    cell_w.saturating_sub(2 * LABEL_GUTTER).max(1)
}

/// Fit a label into at most [`LABEL_MAX_LINES`] lines of `max_w` pixels
/// measured with real glyph widths (see [`crate::text_fit::wrap_lines`]).
pub(crate) fn wrap_label(text: &str, max_w: u32, font_size: u16) -> Vec<String> {
    crate::text_fit::wrap_lines(text, max_w, font_size, LABEL_MAX_LINES)
}

/// Render word-wrapped label lines under an icon.
///
/// Lines are centered within `[cell_x, cell_x + cell_w]` by default. When
/// `icon_center` is `Some(cx)` (column layout) each line is centered on
/// the icon's horizontal midpoint instead, PSIX-style, clamped so long
/// lines stay on-screen.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_label(
    sdi: &mut SdiRegistry,
    at: &ActiveTheme,
    names: &IconNames,
    cell_x: i32,
    cell_w: u32,
    label_y: i32,
    title: &str,
    icon_center: Option<i32>,
    cache: &LabelWrapCache,
) {
    let fs = at.font_small;
    let glyph_h = (fs.max(8) / 8) as u32 * 8;
    let max_w = label_max_width(cell_w);
    let key = (max_w, fs);
    let mut cache = cache.borrow_mut();
    if !matches!(cache.get(title), Some((k, _)) if *k == key) {
        cache.insert(title.to_string(), (key, wrap_label(title, max_w, fs)));
    }
    let lines: &[String] = &cache.get(title).expect("cached above").1;
    let line_h = glyph_h as i32 + 1; // 1px spacing between lines
    let measure = |line: &str| crate::text_fit::text_width(line, fs) as i32;
    // Left edge for a line of pixel width `tw`: centered on the icon
    // midpoint in column layout, otherwise centered within the cell
    // (unchanged legacy arithmetic).
    let line_x = |tw: i32| match icon_center {
        Some(cx) => (cx - tw / 2).max(2),
        None => cell_x + (cell_w as i32 - tw) / 2,
    };

    // Label shadows (1px offset).
    if let Some(shadow_color) = at.icon.label_shadow {
        // Shadow for line 1.
        if let Ok(obj) = sdi.get_mut(&names.shadow) {
            if let Some(line) = lines.first() {
                let tw = measure(line);
                obj.x = line_x(tw) + 1;
                obj.y = label_y + 1;
                obj.w = 0;
                obj.h = 0;
                obj.font_size = fs;
                obj.set_text(line);
                obj.text_color = shadow_color;
                obj.visible = true;
                obj.color = Color::rgba(0, 0, 0, 0);
            } else {
                obj.visible = false;
            }
        }
        // Shadow for line 2.
        if let Ok(obj) = sdi.get_mut(&names.shadow2) {
            if lines.len() > 1 {
                let tw = measure(&lines[1]);
                obj.x = line_x(tw) + 1;
                obj.y = label_y + line_h + 1;
                obj.w = 0;
                obj.h = 0;
                obj.font_size = fs;
                obj.set_text(&lines[1]);
                obj.text_color = shadow_color;
                obj.visible = true;
                obj.color = Color::rgba(0, 0, 0, 0);
            } else {
                obj.visible = false;
            }
        }
    } else {
        if let Ok(obj) = sdi.get_mut(&names.shadow) {
            obj.visible = false;
        }
        if let Ok(obj) = sdi.get_mut(&names.shadow2) {
            obj.visible = false;
        }
    }

    // Line 1.
    if let Ok(obj) = sdi.get_mut(&names.label) {
        if let Some(line) = lines.first() {
            let tw = measure(line);
            obj.x = line_x(tw);
            obj.y = label_y;
            obj.w = 0;
            obj.h = 0;
            obj.font_size = fs;
            obj.set_text(line);
            obj.text_color = at.icon.label_color;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
    // Line 2.
    if let Ok(obj) = sdi.get_mut(&names.label2) {
        if lines.len() > 1 {
            let tw = measure(&lines[1]);
            obj.x = line_x(tw);
            obj.y = label_y + line_h;
            obj.w = 0;
            obj.h = 0;
            obj.font_size = fs;
            obj.set_text(&lines[1]);
            obj.text_color = at.icon.label_color;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::registered_app_titles;
    use crate::text_fit::{ELLIPSIS, text_width};

    #[test]
    fn every_builtin_title_fits_its_cell_at_480x272() {
        for &skin_name in crate::skin::builtin::builtin_names() {
            let skin = crate::skin::builtin::load_builtin(skin_name).expect("builtin skin");
            let at = ActiveTheme::from_skin(&skin.theme)
                .with_screen_size(480, 272)
                .with_features(&skin.features);
            let cfg = super::super::DashboardConfig::from_features(&skin.features, &at);
            let psp_native = skin.manifest.screen_width == 480;
            for cell_w in [cfg.cell_w, cfg.free_cell_w] {
                for title in registered_app_titles() {
                    let lines = wrap_label(title, label_max_width(cell_w), at.font_small);
                    assert!(
                        !lines.is_empty() && lines.len() <= LABEL_MAX_LINES,
                        "{skin_name}: {title:?} -> {lines:?}"
                    );
                    for line in &lines {
                        // Cells of PSP-native skins are sized so no
                        // built-in title needs truncating.
                        assert!(
                            !psp_native || !line.ends_with(ELLIPSIS),
                            "{skin_name}: {title:?} truncated to {line:?} in {cell_w}px"
                        );
                        let w = text_width(line, at.font_small);
                        assert!(
                            w + 2 * LABEL_GUTTER <= cell_w,
                            "{skin_name}: {title:?} line {line:?} is {w}px in a {cell_w}px cell"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_cell_ellipsizes_instead_of_overflowing() {
        let lines = wrap_label("Package Manager", 30, 8);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().any(|l| l.ends_with(ELLIPSIS)), "{lines:?}");
        assert!(lines.iter().all(|l| text_width(l, 8) <= 30));
    }

    #[test]
    fn drawn_labels_stay_inside_their_cells() {
        use super::super::tests::test_config;
        use super::super::{AppEntry, DashboardState};
        let at = ActiveTheme::default();
        let mut cfg = test_config();
        cfg.cell_w = 52;
        cfg.grid_w = 104;
        let apps = ["Network", "Settings", "Package Manager", "System Monitor"]
            .iter()
            .map(|t| AppEntry {
                title: t.to_string(),
                path: format!("/apps/{t}"),
                icon_png: Vec::new(),
                color: Color::WHITE,
            })
            .collect();
        let mut dash = DashboardState::new(cfg, apps);
        let mut sdi = SdiRegistry::new();
        dash.update_sdi(&mut sdi, &at);
        let (cell_w, _) = dash.cell_size();
        for (i, &(cx, _)) in dash.page_cell_origins().iter().enumerate() {
            for name in [format!("icon_label_{i}"), format!("icon_label2_{i}")] {
                let obj = sdi.get(&name).expect("label object");
                if !obj.visible {
                    continue;
                }
                let w = text_width(obj.text.as_deref().unwrap_or(""), obj.font_size) as i32;
                assert!(
                    obj.x >= cx && obj.x + w <= cx + cell_w as i32,
                    "{name} ({:?}) overflows its cell",
                    obj.text
                );
            }
        }
    }
}
