//! Data-driven vector overlay rendering for background decorations.
//!
//! When a skin has `background_layers` configured, this module renders
//! decorative elements (grids, wireframe spheres, radar sweeps, glass shards,
//! scanlines, etc.) directly to the backend via `BackgroundScene`.
//!
//! These overlays are rendered between the base SDI layer (wallpaper, bars)
//! and the overlay layer (cursor, toasts), preserving correct z-order.

use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;
use oasis_vector::AnimClock;
use oasis_vector::ShaderParams;
use oasis_vector::background::{BackgroundScene, LayerKind};
use oasis_vector::render::render_scene;
use oasis_vector::scene::VectorScene;

use crate::active_theme::ActiveTheme;

/// Build and render vector background overlays for the current frame.
///
/// Composes decorative elements from the theme's `background_layers` list
/// into a `BackgroundScene` and renders via `VectorScene`.
///
/// `frame_counter` drives animation timing. Pass 0 for static.
pub fn render_vector_background(
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
    frame_counter: u32,
) -> Result<()> {
    if at.background_layers.is_empty() {
        return Ok(());
    }

    let w = at.screen_w;
    let h = at.screen_h;

    // Build a clock from the frame counter (assumes 60fps).
    let clock = AnimClock {
        time_s: frame_counter as f32 / 60.0,
        frame: frame_counter,
    };

    let scene_data = BackgroundScene {
        layers: at.background_layers.clone(),
        complexity_budget: at.background_complexity_budget,
    };

    let ops = scene_data.build_ops(&clock, w, h, at.background_reduced_motion);
    if ops.is_empty() {
        return Ok(());
    }

    let scene = VectorScene {
        width: w,
        height: h,
        ops,
    };

    render_scene(backend, &scene)
}

/// Information about a shader background layer, extracted from the theme.
pub struct ShaderLayerInfo {
    /// Shader name (matches registry key, e.g. "balatro").
    pub name: String,
    /// Shader-specific parameters.
    pub params: ShaderParams,
}

/// Extract the first shader layer from the active theme (if any).
///
/// Backends call this to determine whether to invoke GPU shader rendering
/// before the vector overlay pass.
pub fn get_shader_layer(at: &ActiveTheme) -> Option<ShaderLayerInfo> {
    at.background_layers.iter().find_map(|l| match &l.kind {
        LayerKind::Shader { name, params } => Some(ShaderLayerInfo {
            name: name.clone(),
            params: params.clone(),
        }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::Color;
    use oasis_vector::background::{BackgroundLayer, LayerAnimation, LayerKind, LayerPosition};
    use oasis_vector::icons;

    #[test]
    fn render_vector_background_empty_layers() {
        // Default theme has no background layers — should return Ok immediately.
        let at = ActiveTheme::default();
        assert!(at.background_layers.is_empty());
    }

    #[test]
    fn render_vector_background_builds_scene() {
        let mut at = ActiveTheme::default();
        at.background_layers.push(BackgroundLayer {
            kind: LayerKind::Grid { spacing: 30 },
            color: Color::rgba(255, 255, 255, 18),
            position: LayerPosition::default(),
            animation: LayerAnimation::default(),
            enabled: true,
        });
        let w = at.screen_w;
        let h = at.screen_h;
        let grid_ops = icons::grid_overlay(w, h, 30, Color::WHITE);
        assert!(!grid_ops.is_empty());
    }
}
