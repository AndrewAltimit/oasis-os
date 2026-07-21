//! TOML parsing helpers and SDI object application.

use std::collections::HashMap;

use serde::Deserialize;

use oasis_sdi::SdiRegistry;

use crate::theme::parse_hex_color;

pub use crate::theme::NinePatchDef;

/// A single SDI object definition in a layout file.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct SkinObjectDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    /// Image asset to render instead of a fill (e.g. `"assets/bar_top.png"`).
    /// The bitmap is alpha-blended, so any silhouette works as shaped chrome.
    /// Uploaded by `Skin::upload_layout_textures` once a backend exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    /// Nine-patch (9-slice) image to render: corners stay fixed, edges and
    /// center stretch to the object's `w`x`h` — scalable chrome (bars,
    /// panels, buttons) from one small bitmap. Takes precedence over
    /// `texture` when both are set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nine_patch: Option<NinePatchDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<i32>,
    // Extended visual properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_top: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_bottom: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_shadow_dx: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_shadow_dy: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_shadow_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_color: Option<String>,
}

/// Layout: a named collection of SDI object definitions (`layout.toml`).
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize))]
pub struct SkinLayout {
    #[serde(flatten)]
    pub objects: HashMap<String, SkinObjectDef>,
}

/// Apply a layout's SDI object definitions to an SDI registry.
pub(crate) fn apply_layout_inner(layout: &SkinLayout, sdi: &mut SdiRegistry, sx: f64, sy: f64) {
    // Create in sorted name order: objects without an explicit `z` get the
    // registry's auto-incrementing z, so HashMap iteration order would make
    // their stacking (and any same-z tiebreak) random per process.
    let mut names: Vec<&String> = layout.objects.keys().collect();
    names.sort();
    for name in names {
        let def = &layout.objects[name];
        if !sdi.contains(name) {
            sdi.create(name);
        }
        if let Ok(obj) = sdi.get_mut(name) {
            if let Some(x) = def.x {
                obj.x = (x as f64 * sx) as i32;
            }
            if let Some(y) = def.y {
                obj.y = (y as f64 * sy) as i32;
            }
            if let Some(w) = def.w {
                obj.w = (w as f64 * sx) as u32;
            }
            if let Some(h) = def.h {
                obj.h = (h as f64 * sy) as u32;
            }
            if let Some(a) = def.alpha {
                obj.alpha = a;
            }
            if let Some(v) = def.visible {
                obj.visible = v;
            }
            if let Some(z) = def.z {
                obj.z = z;
            }
            if let Some(ref t) = def.text {
                obj.text = Some(t.clone());
            }
            if let Some(fs) = def.font_size {
                obj.font_size = fs;
            }
            if let Some(ref c) = def.color
                && let Some(parsed) = parse_hex_color(c)
            {
                obj.color = parsed;
            }
            if let Some(ref c) = def.text_color
                && let Some(parsed) = parse_hex_color(c)
            {
                obj.text_color = parsed;
            }
            // Extended visual properties.
            if let Some(r) = def.border_radius {
                obj.border_radius = Some(r);
            }
            if let Some(ref c) = def.gradient_top
                && let Some(parsed) = parse_hex_color(c)
            {
                obj.gradient_top = Some(parsed);
            }
            if let Some(ref c) = def.gradient_bottom
                && let Some(parsed) = parse_hex_color(c)
            {
                obj.gradient_bottom = Some(parsed);
            }
            if let Some(s) = def.shadow_level {
                obj.shadow_level = Some(s);
            }
            if let Some(sw) = def.stroke_width {
                obj.stroke_width = Some(sw);
            }
            if let Some(ref c) = def.stroke_color
                && let Some(parsed) = parse_hex_color(c)
            {
                obj.stroke_color = Some(parsed);
            }
            if def.text_shadow_dx.is_some() || def.text_shadow_dy.is_some() {
                obj.text_shadow_offset = Some((
                    def.text_shadow_dx.unwrap_or(1),
                    def.text_shadow_dy.unwrap_or(1),
                ));
            }
            if let Some(ref c) = def.text_shadow_color
                && let Some(parsed) = parse_hex_color(c)
            {
                obj.text_shadow_color = Some(parsed);
            }
            if let Some(ref c) = def.shadow_color
                && let Some(parsed) = parse_hex_color(c)
            {
                obj.shadow_color = Some(parsed);
            }
        }
    }
}
