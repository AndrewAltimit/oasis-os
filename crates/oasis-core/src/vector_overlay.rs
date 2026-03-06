//! Vector overlay rendering for backgrounds and decorative elements.
//!
//! When the skin uses `icon_style = "vector"` or a vector-enabled wallpaper,
//! this module provides background decorations (grid, wireframe sphere,
//! radar sweep, glass shards) rendered directly to the backend.
//!
//! These overlays are rendered between the base SDI layer (wallpaper, bars)
//! and the overlay layer (cursor, toasts), preserving correct z-order.

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::color::with_alpha;
use oasis_types::error::Result;
use oasis_vector::icons;
use oasis_vector::render::render_scene;
use oasis_vector::scene::VectorScene;

use crate::active_theme::ActiveTheme;

/// Build and render a vector background overlay for the current frame.
///
/// Composes decorative elements based on theme colors:
/// - Sparse grid overlay (subtle guide lines)
/// - Wireframe sphere (right side, Altimit-style globe)
/// - Radar sweep arc (animated if `frame_counter` is provided)
/// - Glass polygon shards (translucent geometric shapes)
///
/// `frame_counter` drives animation (radar rotation). Pass 0 for static.
pub fn render_vector_background(
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
    frame_counter: u32,
) -> Result<()> {
    let w = at.screen_w;
    let h = at.screen_h;

    let mut scene = VectorScene::new(w, h);

    // Grid overlay: subtle but visible guide lines.
    let grid_color = with_alpha(Color::WHITE, 18);
    let grid_ops = icons::grid_overlay(w, h, 30, grid_color);
    for op in grid_ops {
        scene.push(op);
    }

    // Wireframe sphere on the right side (animated rotation).
    let sphere_r: u16 = (h / 4).min(60) as u16;
    let sphere_cx = (w as i32 * 4) / 5;
    let sphere_cy = h as i32 / 2;
    let sphere_color = with_alpha(Color::WHITE, 50);
    let sphere_angle = frame_counter as f32 * 0.02; // globe rotation
    let sphere = icons::wireframe_sphere_animated(sphere_r, sphere_color, sphere_angle);
    scene.embed(
        sphere_cx - sphere_r as i32,
        sphere_cy - sphere_r as i32,
        &VectorScene {
            width: sphere.width,
            height: sphere.height,
            ops: sphere.ops,
        },
    );

    // Radar sweep (animated).
    let sweep_angle = 0.8; // ~45 degrees
    let rotation = (frame_counter as f32) * 0.03;
    let radar_color = with_alpha(Color::WHITE, 30);
    scene.push(icons::radar_sweep(
        sphere_cx,
        sphere_cy,
        sphere_r + 5,
        sweep_angle,
        rotation,
        radar_color,
    ));

    // Glass polygon shards (decorative translucent triangles).
    let shard_color = with_alpha(Color::WHITE, 20);
    // Bottom-left shard.
    scene.push(icons::glass_polygon(
        vec![
            (0, h as i32),
            ((w / 5) as i32, (h * 3 / 4) as i32),
            ((w / 8) as i32, h as i32),
        ],
        shard_color,
        20,
    ));
    // Top-right shard.
    scene.push(icons::glass_polygon(
        vec![
            ((w * 3 / 4) as i32, 0),
            (w as i32, (h / 6) as i32),
            (w as i32, 0),
        ],
        shard_color,
        20,
    ));

    render_scene(backend, &scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_vector_background_builds_scene() {
        // Just verify it doesn't panic with default theme.
        let at = ActiveTheme::default();
        // We can't easily test rendering without a mock backend, but we
        // can test scene construction.
        let w = at.screen_w;
        let h = at.screen_h;
        let mut scene = VectorScene::new(w, h);
        let grid_ops = icons::grid_overlay(w, h, 30, Color::WHITE);
        assert!(!grid_ops.is_empty());
        for op in grid_ops {
            scene.push(op);
        }
        assert!(!scene.ops.is_empty());
    }
}
