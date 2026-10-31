//! SDI object creation, destruction, and position updates for windows.
//!
//! Kept apart from `manager/` to separate rendering concerns from
//! window lifecycle and input handling logic.

use oasis_sdi::SdiRegistry;
use oasis_types::backend::Color;

use super::hit_test::{ButtonKind, HitRegion, hit_test};
use super::manager::{MODAL_OVERLAY_ID, WindowManager};
use super::window::{Window, WmTheme};

/// Titlebar buttons in creation order.
const BUTTON_KINDS: [ButtonKind; 3] = [
    ButtonKind::Close,
    ButtonKind::Minimize,
    ButtonKind::Maximize,
];

/// Per-button SDI names and theme colors.
struct ButtonStyle<'a> {
    /// SDI suffix of the button background.
    suffix: &'static str,
    /// SDI suffix of the button's glyph text.
    glyph_suffix: &'static str,
    color: Color,
    hover: Color,
    glyph: &'a str,
    glyph_color: Color,
}

impl<'a> ButtonStyle<'a> {
    fn of(theme: &'a WmTheme, kind: ButtonKind) -> Self {
        match kind {
            ButtonKind::Close => Self {
                suffix: "btn_close",
                glyph_suffix: "btn_close_glyph",
                color: theme.btn_close_color,
                hover: theme.btn_close_hover,
                glyph: &theme.glyph_close,
                glyph_color: theme.glyph_close_color,
            },
            ButtonKind::Minimize => Self {
                suffix: "btn_minimize",
                glyph_suffix: "btn_minimize_glyph",
                color: theme.btn_minimize_color,
                hover: theme.btn_minimize_hover,
                glyph: &theme.glyph_minimize,
                glyph_color: theme.glyph_minimize_color,
            },
            ButtonKind::Maximize => Self {
                suffix: "btn_maximize",
                glyph_suffix: "btn_maximize_glyph",
                color: theme.btn_maximize_color,
                hover: theme.btn_maximize_hover,
                glyph: &theme.glyph_maximize,
                glyph_color: theme.glyph_maximize_color,
            },
        }
    }
}

/// Font size of the titlebar button glyphs.
fn glyph_font_size(theme: &WmTheme) -> u16 {
    theme.button_size.min(12) as u16
}

/// `pos + (span - size) / 2`, saturating (positions near the ends of the
/// i32 range must not overflow).
fn centered(pos: i32, span: u32, size: u32) -> i32 {
    let off = (i64::from(span) - i64::from(size)) / 2;
    (i64::from(pos) + off).clamp(i32::MIN.into(), i32::MAX.into()) as i32
}

/// Top-left of a `size`-pixel glyph centered in button `rect`.
fn glyph_origin(rect: (i32, i32, u32, u32), size: u16) -> (i32, i32) {
    let (bx, by, bw, bh) = rect;
    let size = u32::from(size);
    (centered(bx, bw, size), centered(by, bh, size))
}

/// Y of the title text in a titlebar at `ty` of height `th`.
fn title_text_y(ty: i32, th: u32, theme: &WmTheme) -> i32 {
    centered(ty, th, u32::from(theme.titlebar_font_size)).saturating_sub(1)
}

/// Y of the 1px separator along the bottom edge of a titlebar.
fn separator_y(ty: i32, th: u32) -> i32 {
    (i64::from(ty) + i64::from(th) - 1).clamp(i32::MIN.into(), i32::MAX.into()) as i32
}

impl WindowManager {
    /// Create all SDI objects for a window.
    pub(crate) fn create_sdi_objects(&self, window: &super::window::Window, sdi: &mut SdiRegistry) {
        let theme = &self.theme;
        let use_radius = theme.frame_border_radius > 0;
        let use_shadow = theme.frame_shadow_level > 0;

        // Frame (background).
        if window.sdi_suffixes().contains(&"frame") {
            let obj = sdi.create(window.sdi_name("frame"));
            obj.x = window.x;
            obj.y = window.y;
            obj.w = window.outer_w;
            obj.h = window.outer_h;
            obj.color = theme.frame_color;
            if use_radius {
                obj.border_radius = Some(theme.frame_border_radius);
            }
            if use_shadow {
                obj.shadow_level = Some(theme.frame_shadow_level);
            }
            if theme.border_width > 0 {
                obj.stroke_width = Some(theme.border_width as u16);
                obj.stroke_color = Some(theme.frame_color);
            }
            if let Some((tex, slices)) = theme.frame_patch {
                obj.texture = Some(tex);
                obj.nine_patch = Some(slices);
            }
        }

        // Titlebar.
        if let Some((tx, ty, tw, th)) = window.titlebar_rect(theme) {
            if window.sdi_suffixes().contains(&"titlebar") {
                let obj = sdi.create(window.sdi_name("titlebar"));
                obj.x = tx;
                obj.y = ty;
                obj.w = tw;
                obj.h = th;
                obj.color = theme.titlebar_active_color;
                if theme.titlebar_radius > 0 {
                    obj.border_radius = Some(theme.titlebar_radius);
                }
                if theme.titlebar_gradient {
                    use oasis_types::color::lighten;
                    obj.gradient_top = Some(
                        theme
                            .titlebar_gradient_top
                            .unwrap_or_else(|| lighten(theme.titlebar_active_color, 0.1)),
                    );
                    obj.gradient_bottom = Some(
                        theme
                            .titlebar_gradient_bottom
                            .unwrap_or(theme.titlebar_active_color),
                    );
                }
                if let Some((tex, slices)) = theme.titlebar_patch {
                    obj.texture = Some(tex);
                    obj.nine_patch = Some(slices);
                }
            }

            // Title text.
            if window.sdi_suffixes().contains(&"title_text")
                && let Some(layout) = window.title_layout(theme)
            {
                let text_x = layout.x;
                let avail_w = layout.text_w;
                let text_y = title_text_y(ty, th, theme);
                let obj = sdi.create(window.sdi_name("title_text"));
                obj.x = text_x;
                obj.y = text_y;
                obj.w = avail_w;
                obj.h = th;
                obj.text = Some(layout.text.clone());
                obj.font_size = theme.titlebar_font_size;
                obj.text_color = theme.titlebar_text_color;
                obj.color = Color::rgba(0, 0, 0, 0);

                // Title text shadow (Tier 3).
                if theme.title_text_shadow {
                    let sobj = sdi.create(window.sdi_name("title_shadow"));
                    sobj.x = text_x.saturating_add(1);
                    sobj.y = text_y.saturating_add(1);
                    sobj.w = avail_w;
                    sobj.h = th;
                    sobj.text = Some(layout.text);
                    sobj.font_size = theme.titlebar_font_size;
                    sobj.text_color = theme.title_text_shadow_color;
                    sobj.color = Color::rgba(0, 0, 0, 0);
                }
            }

            // Separator (Tier 2): 1px bar at titlebar bottom edge.
            if theme.separator_enabled {
                let obj = sdi.create(window.sdi_name("separator"));
                obj.x = tx;
                obj.y = separator_y(ty, th);
                obj.w = tw;
                obj.h = 1;
                obj.color = theme.separator_color;
            }
        }

        // Titlebar buttons + glyphs.
        let glyph_size = glyph_font_size(theme);
        for kind in BUTTON_KINDS {
            let Some(rect) = window.button_rect(theme, kind) else {
                continue;
            };
            let style = ButtonStyle::of(theme, kind);
            let obj = sdi.create(window.sdi_name(style.suffix));
            (obj.x, obj.y, obj.w, obj.h) = rect;
            obj.color = style.color;
            if theme.button_radius > 0 {
                obj.border_radius = Some(theme.button_radius);
            }
            let gobj = sdi.create(window.sdi_name(style.glyph_suffix));
            (gobj.x, gobj.y) = glyph_origin(rect, glyph_size);
            (gobj.w, gobj.h) = (rect.2, rect.3);
            gobj.text = Some(style.glyph.to_string());
            gobj.font_size = glyph_size;
            gobj.text_color = style.glyph_color;
            gobj.color = Color::rgba(0, 0, 0, 0);
        }

        // Content area (stays sharp for clip rect compatibility).
        {
            let (cx, cy, cw, ch) = window.content_rect(theme);
            let obj = sdi.create(window.sdi_name("content"));
            obj.x = cx;
            obj.y = cy;
            obj.w = cw;
            obj.h = ch;
            obj.color = theme.content_bg_color;

            // Content stroke overlay (Tier 3).
            if theme.content_stroke_width > 0 {
                let sobj = sdi.create(window.sdi_name("content_stroke"));
                sobj.x = cx;
                sobj.y = cy;
                sobj.w = cw;
                sobj.h = ch;
                sobj.color = Color::rgba(0, 0, 0, 0);
                sobj.stroke_width = Some(theme.content_stroke_width);
                sobj.stroke_color = Some(theme.content_stroke_color);
            }
        }
    }

    /// Rebuild every window's SDI objects from the current theme. Call
    /// after [`WindowManager::set_theme`]: chrome colors, sizes, radii and
    /// fonts are baked into the objects when a window is created, so open
    /// windows would otherwise keep the previous skin's look. Geometry,
    /// state (minimized / kiosk), z-order and focus are kept; running
    /// open/minimize animations are finished and closing ghosts dropped.
    pub fn restyle_windows(&mut self, sdi: &mut SdiRegistry) {
        let ids: Vec<_> = self.windows.iter().map(|w| w.id.clone()).collect();
        for id in &ids {
            if self.anim.is_animating(id) {
                self.finish_animation(id, sdi);
            }
        }
        self.flush_closing(None, sdi);
        self.hover_button = None;
        for window in &self.windows {
            self.destroy_sdi_objects(window, sdi);
            self.create_sdi_objects(window, sdi);
            self.layout_window_sdi(window, sdi);
            let minimized = window.state == super::window::WindowState::Minimized;
            for suffix in window.sdi_suffixes() {
                if (minimized || (window.fullscreen_kiosk && *suffix != "content"))
                    && let Ok(obj) = sdi.get_mut(&window.sdi_name(suffix))
                {
                    obj.visible = false;
                }
            }
        }
        // Restore z-order (new objects were appended on top) and the
        // active / inactive titlebar colors.
        match self.active_window.clone() {
            Some(active) => self.focus_window_internal(&active, sdi),
            None => {
                for window in &self.windows {
                    for suffix in window.sdi_suffixes() {
                        let _ = sdi.move_to_top(&window.sdi_name(suffix));
                    }
                }
            },
        }
    }

    /// Destroy all SDI objects for a window.
    pub(crate) fn destroy_sdi_objects(
        &self,
        window: &super::window::Window,
        sdi: &mut SdiRegistry,
    ) {
        for suffix in window.sdi_suffixes() {
            let name = window.sdi_name(suffix);
            let _ = sdi.destroy(&name);
        }
    }

    /// Reposition all SDI objects based on window's current geometry.
    ///
    /// Any open/minimize/restore animation running on the window is
    /// finished first: a logical geometry change supersedes it.
    pub(crate) fn update_sdi_positions(&mut self, id: &str, sdi: &mut SdiRegistry) {
        if self.anim.is_animating(id) {
            self.finish_animation(id, sdi);
        }
        if let Some(window) = self.windows.iter().find(|w| w.id == id) {
            self.layout_window_sdi(window, sdi);
        }
    }

    /// Position every SDI object of `window` from its current geometry.
    ///
    /// Animations call this with a temporary copy of the window carrying
    /// the interpolated geometry.
    pub(crate) fn layout_window_sdi(&self, window: &super::window::Window, sdi: &mut SdiRegistry) {
        let theme = &self.theme;

        // Frame.
        if let Ok(obj) = sdi.get_mut(&window.sdi_name("frame")) {
            obj.x = window.x;
            obj.y = window.y;
            obj.w = window.outer_w;
            obj.h = window.outer_h;
        }

        // Titlebar.
        if let Some((tx, ty, tw, th)) = window.titlebar_rect(theme) {
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("titlebar")) {
                obj.x = tx;
                obj.y = ty;
                obj.w = tw;
                obj.h = th;
            }
            // Title text: re-layout, since a new width can change both the
            // centered position and the ellipsis truncation.
            if let Some(layout) = window.title_layout(theme) {
                let text_y = title_text_y(ty, th, theme);
                for (suffix, off) in [("title_text", 0), ("title_shadow", 1)] {
                    if let Ok(obj) = sdi.get_mut(&window.sdi_name(suffix)) {
                        obj.x = layout.x.saturating_add(off);
                        obj.y = text_y.saturating_add(off);
                        obj.w = layout.text_w;
                        obj.h = th;
                        if obj.text.as_deref() != Some(layout.text.as_str()) {
                            obj.text = Some(layout.text.clone());
                        }
                    }
                }
            }
            // Separator.
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("separator")) {
                obj.x = tx;
                obj.y = separator_y(ty, th);
                obj.w = tw;
                obj.h = 1;
            }
        }

        // Buttons + glyphs.
        let glyph_size = glyph_font_size(theme);
        for kind in BUTTON_KINDS {
            let Some(rect) = window.button_rect(theme, kind) else {
                continue;
            };
            let style = ButtonStyle::of(theme, kind);
            if let Ok(obj) = sdi.get_mut(&window.sdi_name(style.suffix)) {
                (obj.x, obj.y, obj.w, obj.h) = rect;
            }
            if let Ok(obj) = sdi.get_mut(&window.sdi_name(style.glyph_suffix)) {
                (obj.x, obj.y) = glyph_origin(rect, glyph_size);
            }
        }

        // Content.
        let (cx, cy, cw, ch) = window.content_rect(theme);
        if let Ok(obj) = sdi.get_mut(&window.sdi_name("content")) {
            obj.x = cx;
            obj.y = cy;
            obj.w = cw;
            obj.h = ch;
        }
        // Content stroke.
        if let Ok(obj) = sdi.get_mut(&window.sdi_name("content_stroke")) {
            obj.x = cx;
            obj.y = cy;
            obj.w = cw;
            obj.h = ch;
        }
    }

    /// Re-apply nine-patch chrome textures to every existing window's SDI
    /// objects. The shell calls this after uploading (or clearing) the
    /// theme's `titlebar_patch` / `frame_patch` on a skin swap; new windows
    /// pick the patches up in `create_sdi_objects`.
    pub fn apply_chrome_patches(&self, sdi: &mut SdiRegistry) {
        for window in &self.windows {
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("titlebar")) {
                match self.theme.titlebar_patch {
                    Some((tex, slices)) => {
                        obj.texture = Some(tex);
                        obj.nine_patch = Some(slices);
                    },
                    None => {
                        obj.texture = None;
                        obj.nine_patch = None;
                    },
                }
            }
            if let Ok(obj) = sdi.get_mut(&window.sdi_name("frame")) {
                match self.theme.frame_patch {
                    Some((tex, slices)) => {
                        obj.texture = Some(tex);
                        obj.nine_patch = Some(slices);
                    },
                    None => {
                        obj.texture = None;
                        obj.nine_patch = None;
                    },
                }
            }
        }
    }

    /// Update button hover state. Sets hover color on the hovered button and
    /// restores the base color on the previously hovered button.
    pub(crate) fn update_button_hover(&mut self, x: i32, y: i32, sdi: &mut SdiRegistry) {
        let region = hit_test(&self.windows, x, y, &self.theme);
        let modal = self.topmost_modal();
        let new_hover = match &region {
            // Under a modal only its own buttons react (as for clicks).
            HitRegion::TitlebarButton(id, kind) if modal.is_none_or(|m| m == *id) => {
                Some((id.clone(), *kind))
            },
            _ => None,
        };

        // If hover changed, restore old button color.
        if self.hover_button != new_hover {
            if let Some((ref old_id, old_kind)) = self.hover_button {
                let style = ButtonStyle::of(&self.theme, old_kind);
                if let Ok(obj) = sdi.get_mut(&format!("{old_id}.{}", style.suffix)) {
                    obj.color = style.color;
                }
            }
            // Apply new hover color.
            if let Some((ref new_id, new_kind)) = new_hover {
                let style = ButtonStyle::of(&self.theme, new_kind);
                if let Ok(obj) = sdi.get_mut(&format!("{new_id}.{}", style.suffix)) {
                    obj.color = style.hover;
                }
            }
            self.hover_button = new_hover;
        }
    }

    /// Show the semi-transparent overlay behind modal windows.
    pub(crate) fn show_modal_overlay(&self, sdi: &mut SdiRegistry) {
        if sdi.contains(MODAL_OVERLAY_ID) {
            return;
        }
        let obj = sdi.create(MODAL_OVERLAY_ID.to_string());
        obj.x = 0;
        obj.y = 0;
        obj.w = self.screen_w;
        obj.h = self.screen_h;
        obj.color = self.theme.modal_overlay_color;
    }

    /// Hide and destroy the modal overlay.
    pub(crate) fn hide_modal_overlay(&self, sdi: &mut SdiRegistry) {
        let _ = sdi.destroy(MODAL_OVERLAY_ID);
    }
}

/// Show or hide the SDI objects of `window`.
///
/// Hiding hides everything. Showing a window in fullscreen kiosk mode
/// shows only its `content` object: its decorations stay hidden for as long
/// as it is in kiosk mode.
pub(crate) fn set_window_visible(window: &Window, visible: bool, sdi: &mut SdiRegistry) {
    for &suffix in window.sdi_suffixes() {
        let show = visible && (!window.fullscreen_kiosk || suffix == "content");
        if let Ok(obj) = sdi.get_mut(&window.sdi_name(suffix)) {
            obj.visible = show;
        }
    }
}
