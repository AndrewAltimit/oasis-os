//! Rendering and framebuffer access: `oasis_tick`, `oasis_get_buffer`, `oasis_get_dirty`.

use oasis_core::backend::{InputBackend, SdiCore};
use oasis_core::input::{Button, InputEvent, Trigger};
use oasis_core::platform::{PowerService, TimeService};

use crate::handle::{OasisInstance, with_instance, with_instance_ref};
use crate::types::OASIS_CB_APP_LAUNCH;

/// Nearest-neighbor upscale `src` (sw x sh RGBA) into `dst` (dw x dh RGBA),
/// resizing `dst` as needed. Used to upscale the reduced-res shader wallpaper.
fn upscale_nearest(src: &[u8], sw: u32, sh: u32, dst: &mut Vec<u8>, dw: u32, dh: u32) {
    let needed = (dw as usize) * (dh as usize) * 4;
    if dst.len() != needed {
        dst.resize(needed, 0);
    }
    if sw == 0 || sh == 0 {
        return;
    }
    // The source must hold `sw * sh` RGBA pixels; a short buffer would panic on the
    // `src[s..s + 4]` slice below. Catch renderer/dimension mismatches in debug builds.
    debug_assert!(src.len() >= (sw as usize) * (sh as usize) * 4);
    for dy in 0..dh {
        // Widen to u64: `dy * sh` can overflow u32 for pathological FFI dimensions.
        let sy = ((dy as u64 * sh as u64 / dh as u64) as u32).min(sh - 1);
        let src_row = (sy as usize) * (sw as usize) * 4;
        let dst_row = (dy as usize) * (dw as usize) * 4;
        for dx in 0..dw {
            let sx = ((dx as u64 * sw as u64 / dw as u64) as u32).min(sw - 1);
            let s = src_row + (sx as usize) * 4;
            let d = dst_row + (dx as usize) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

/// Advance the OS state by one frame.
///
/// Processes queued input events and updates the scene graph.
///
/// # Safety
///
/// `handle` must be a valid, non-null instance pointer.
///
/// # Thread Safety
///
/// Caller must ensure single-threaded access to the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_tick(handle: *mut OasisInstance, delta_seconds: f32) {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, (), |instance| {
            tick_inner(instance, delta_seconds);
        });
    }
}

fn tick_inner(instance: &mut OasisInstance, delta_seconds: f32) {
    instance.shader_time += delta_seconds;

    // Process queued input events.
    let events = instance.input.poll_events();

    // Collect callback details to fire after releasing dashboard borrow.
    let mut pending_callbacks: Vec<(u32, String)> = Vec::new();

    for event in &events {
        match event {
            InputEvent::ButtonPress(btn) => {
                if let Some(ref mut dashboard) = instance.dashboard {
                    match btn {
                        Button::Up | Button::Down | Button::Left | Button::Right => {
                            dashboard.handle_input(btn);
                        },
                        Button::Confirm => {
                            if let Some(app) = dashboard.selected_app() {
                                pending_callbacks.push((OASIS_CB_APP_LAUNCH, app.title.clone()));
                            }
                        },
                        _ => {},
                    }
                }
            },
            InputEvent::TriggerPress(Trigger::Right) => {
                if let Some(ref mut dashboard) = instance.dashboard {
                    dashboard.next_page();
                }
            },
            InputEvent::TriggerPress(Trigger::Left) => {
                if let Some(ref mut dashboard) = instance.dashboard {
                    dashboard.prev_page();
                }
            },
            _ => {},
        }
    }

    // Fire pending callbacks outside the dashboard borrow scope.
    for (event, detail) in &pending_callbacks {
        instance.fire_callback(*event, detail);
    }

    // Update SDI.
    if let Some(ref mut dashboard) = instance.dashboard {
        dashboard.update_sdi(&mut instance.sdi, &instance.active_theme);
    }

    // Draw the status bar (top), bottom bar (clock/date), and start button -- the
    // chrome the desktop app renders around the dashboard. Without these the skin's
    // reserved top/bottom strips stay empty. Only meaningful when a skin is loaded.
    if let Some(ref skin) = instance.skin {
        // Feed real wall-clock time so the bars show the live time/date instead of
        // the "--:--" placeholders. `update_info` must run before `update_sdi`.
        let time = instance.platform.now().ok();
        let power = instance.platform.power_info().ok();
        instance
            .status_bar
            .update_info(time.as_ref(), power.as_ref());
        instance.bottom_bar.update_info(time.as_ref());

        instance
            .status_bar
            .update_sdi(&mut instance.sdi, &instance.active_theme, &skin.features);
        instance
            .bottom_bar
            .update_sdi(&mut instance.sdi, &instance.active_theme, &skin.features);
        if skin.features.start_menu {
            instance
                .start_menu
                .update_sdi(&mut instance.sdi, &instance.active_theme);
        }
    }

    // --- Render scheduler ---------------------------------------------------
    // Skip the (expensive) render entirely when nothing visible changed: no input
    // this tick, and not yet time for either the animated-wallpaper frame or the
    // periodic refresh that catches the clock. Skipping leaves the framebuffer
    // untouched, so the backend stays "not dirty" and the host skips its texture
    // upload too. Input always forces an immediate render for responsiveness.
    let shader_layer = oasis_core::vector_overlay::get_shader_layer(&instance.active_theme);
    let has_shader = shader_layer.is_some();
    const SHADER_FPS: f32 = 12.0; // animated wallpaper refresh rate
    const STATIC_FPS: f32 = 4.0; // idle desktop refresh rate (enough to catch the clock)
    let interval = if has_shader {
        1.0 / SHADER_FPS
    } else {
        1.0 / STATIC_FPS
    };
    let had_input = !events.is_empty();
    let should_render = had_input || (instance.shader_time - instance.last_render_time) >= interval;
    if !should_render {
        return;
    }
    instance.last_render_time = instance.shader_time;

    // Render.
    let _ = instance
        .backend
        .clear(oasis_core::backend::Color::rgb(10, 10, 18));

    // Shader wallpaper FIRST (replaces bg clear). Rendered at quarter resolution
    // and nearest-upscaled -- the dominant render cost, and quartering it only
    // softens the background. The scheduler above already caps it to SHADER_FPS.
    if let Some(info) = shader_layer {
        let full_w = instance.width;
        let full_h = instance.height;
        let qw = (full_w / 4).max(1);
        let qh = (full_h / 4).max(1);
        let renderer = instance
            .software_shader
            .get_or_insert_with(|| oasis_shader::software::SoftwareShaderRenderer::new(qw, qh));
        let pixels = renderer
            .render_shader(&info.name, instance.shader_time, &info.params)
            .to_vec();
        upscale_nearest(&pixels, qw, qh, &mut instance.shader_cache, full_w, full_h);
        instance
            .backend
            .blit_rgba(0, 0, full_w, full_h, &instance.shader_cache);
    }

    if instance.active_theme.icon.style == "vector"
        || !instance.active_theme.background_layers.is_empty()
    {
        let _ = instance.sdi.draw_base_layer(&mut instance.backend);

        let _ = oasis_core::vector_overlay::render_vector_background(
            &mut instance.backend,
            &instance.active_theme,
            0,
        );
        if let Some(ref dash) = instance.dashboard {
            let _ = dash.render_vector_icons(&mut instance.backend, &instance.active_theme, 0);
        }
        let _ = instance.sdi.draw_overlay_layer(&mut instance.backend);
    } else {
        let _ = instance.sdi.draw(&mut instance.backend);
    }

    let _ = instance.backend.swap_buffers();
}

/// Get a pointer to the RGBA framebuffer.
///
/// Writes the buffer dimensions to `out_width` and `out_height` if non-null.
/// The returned pointer is valid until the next `oasis_tick` or `oasis_destroy`.
///
/// # Safety
///
/// `handle` must be valid. `out_width` and `out_height` may be null.
///
/// # Thread Safety
///
/// Caller must ensure single-threaded access to the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_get_buffer(
    handle: *mut OasisInstance,
    out_width: *mut u32,
    out_height: *mut u32,
) -> *const u8 {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance_ref(handle, std::ptr::null(), |instance| {
            // SAFETY: Pointer is either null (handled by `as_mut()` returning None)
            // or valid per caller.
            if let Some(w) = out_width.as_mut() {
                *w = instance.width;
            }
            // SAFETY: Pointer is either null (handled by `as_mut()` returning None)
            // or valid per caller.
            if let Some(h) = out_height.as_mut() {
                *h = instance.height;
            }
            instance.backend.buffer().as_ptr()
        })
    }
}

/// Check whether the framebuffer has changed since the last read.
///
/// # Safety
///
/// `handle` must be valid and non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oasis_get_dirty(handle: *mut OasisInstance) -> bool {
    // SAFETY: Caller guarantees `handle` is valid and non-null per function safety contract.
    unsafe {
        with_instance(handle, false, |instance| {
            let dirty = instance.backend.is_dirty();
            if dirty {
                instance.backend.clear_dirty();
            }
            dirty
        })
    }
}
