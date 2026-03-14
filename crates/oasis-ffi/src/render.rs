//! Rendering and framebuffer access: `oasis_tick`, `oasis_get_buffer`, `oasis_get_dirty`.

use oasis_core::backend::{InputBackend, SdiCore};
use oasis_core::input::{Button, InputEvent, Trigger};

use crate::handle::{OasisInstance, with_instance, with_instance_ref};
use crate::types::OASIS_CB_APP_LAUNCH;

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

    // Render.
    let _ = instance
        .backend
        .clear(oasis_core::backend::Color::rgb(10, 10, 18));

    // Render shader wallpaper FIRST (replaces bg clear).
    if let Some(info) = oasis_core::vector_overlay::get_shader_layer(&instance.active_theme) {
        let renderer = instance.software_shader.get_or_insert_with(|| {
            oasis_shader::software::SoftwareShaderRenderer::new(instance.width, instance.height)
        });
        let pixels = renderer.render_shader(&info.name, instance.shader_time, &info.params);
        instance
            .backend
            .blit_rgba(0, 0, instance.width, instance.height, pixels);
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
