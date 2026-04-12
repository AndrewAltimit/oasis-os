//! Ordering tests for `SdiRenderTarget` on `RecordingBackend`.
//!
//! PR2 of the compositor overhaul (see
//! `docs/compositor-overhaul-plan.md`). The browser compositor emits a
//! canonical sequence of render-target commands per compositing layer.
//! These tests lock that sequence down so regressions in the trait
//! surface or in the recording backend are caught immediately.

use oasis_test_backend::RecordingBackend;
use oasis_types::backend::{
    BlendMode, Color, DrawCommand, RenderTargetId, SdiCore, SdiRenderTarget,
};

/// Walk the canonical single-layer recording:
/// create → bind → fill → text → unbind → composite → destroy.
#[test]
fn canonical_single_layer_sequence() {
    let mut b = RecordingBackend::new(480, 272);

    let id = b.create_render_target(64, 48).expect("create");
    b.bind_render_target(id).expect("bind");
    b.fill_rect(0, 0, 64, 48, Color::rgb(10, 20, 30))
        .expect("fill_rect");
    b.draw_text("hi", 4, 4, 12, Color::WHITE)
        .expect("draw_text");
    b.unbind_render_target().expect("unbind");
    b.composite_render_target(id, 10, 20, 64, 48, BlendMode::Multiply, 0.75)
        .expect("composite");
    b.destroy_render_target(id).expect("destroy");

    let cmds = b.commands();
    assert_eq!(cmds.len(), 7, "got {cmds:#?}");

    assert!(matches!(
        cmds[0],
        DrawCommand::CreateRenderTarget {
            id: RenderTargetId(1),
            w: 64,
            h: 48,
        }
    ));
    assert!(matches!(
        cmds[1],
        DrawCommand::BindRenderTarget {
            id: RenderTargetId(1),
        }
    ));
    assert!(matches!(cmds[2], DrawCommand::FillRect { .. }));
    assert!(matches!(cmds[3], DrawCommand::DrawText { .. }));
    assert!(matches!(cmds[4], DrawCommand::UnbindRenderTarget));
    match cmds[5] {
        DrawCommand::CompositeRenderTarget {
            id,
            dst_x,
            dst_y,
            dst_w,
            dst_h,
            blend,
            opacity,
        } => {
            assert_eq!(id, RenderTargetId(1));
            assert_eq!((dst_x, dst_y, dst_w, dst_h), (10, 20, 64, 48));
            assert_eq!(blend, BlendMode::Multiply);
            assert!((opacity - 0.75).abs() < 1e-6);
        },
        ref other => panic!("expected CompositeRenderTarget, got {other:?}"),
    }
    assert!(matches!(
        cmds[6],
        DrawCommand::DestroyRenderTarget {
            id: RenderTargetId(1),
        }
    ));

    // After the teardown the bind stack is empty and no render targets
    // remain live.
    assert_eq!(b.render_target_bind_depth(), 0);
    assert_eq!(b.live_render_target_count(), 0);
}

/// Three-level nesting models
/// `mix-blend-mode` inside `backdrop-filter` inside `isolation`.
#[test]
fn three_level_nesting_preserves_bind_stack() {
    let mut b = RecordingBackend::new(480, 272);

    let outer = b.create_render_target(256, 256).expect("outer");
    let middle = b.create_render_target(128, 128).expect("middle");
    let inner = b.create_render_target(64, 64).expect("inner");

    b.bind_render_target(outer).expect("bind outer");
    assert_eq!(b.currently_bound_render_target(), Some(outer));
    assert_eq!(b.render_target_bind_depth(), 1);

    b.bind_render_target(middle).expect("bind middle");
    assert_eq!(b.currently_bound_render_target(), Some(middle));
    assert_eq!(b.render_target_bind_depth(), 2);

    b.bind_render_target(inner).expect("bind inner");
    assert_eq!(b.currently_bound_render_target(), Some(inner));
    assert_eq!(b.render_target_bind_depth(), 3);

    b.unbind_render_target().expect("unbind inner");
    assert_eq!(b.currently_bound_render_target(), Some(middle));

    b.unbind_render_target().expect("unbind middle");
    assert_eq!(b.currently_bound_render_target(), Some(outer));

    b.unbind_render_target().expect("unbind outer");
    assert_eq!(b.currently_bound_render_target(), None);
    assert_eq!(b.render_target_bind_depth(), 0);

    // All three targets are still live -- `unbind_render_target` does
    // not destroy, only pops the stack.
    assert_eq!(b.live_render_target_count(), 3);

    b.destroy_render_target(outer).expect("destroy outer");
    b.destroy_render_target(middle).expect("destroy middle");
    b.destroy_render_target(inner).expect("destroy inner");
    assert_eq!(b.live_render_target_count(), 0);
}

/// `PushCompositingLayer` / `PopCompositingLayer` in the display list
/// maps to the canonical sequence below. The replayer can be verified
/// against this by constructing the command stream manually.
#[test]
fn pop_layer_emits_composite_before_destroy() {
    let mut b = RecordingBackend::new(480, 272);

    let id = b.create_render_target(100, 100).expect("create");
    b.bind_render_target(id).expect("bind");
    b.unbind_render_target().expect("unbind");
    // PopCompositingLayer lowers to composite → destroy (never the
    // reverse).
    b.composite_render_target(id, 0, 0, 100, 100, BlendMode::Screen, 1.0)
        .expect("composite");
    b.destroy_render_target(id).expect("destroy");

    let cmds = b.commands();
    let composite_idx = cmds
        .iter()
        .position(|c| matches!(c, DrawCommand::CompositeRenderTarget { .. }))
        .expect("composite present");
    let destroy_idx = cmds
        .iter()
        .position(|c| matches!(c, DrawCommand::DestroyRenderTarget { .. }))
        .expect("destroy present");
    assert!(
        composite_idx < destroy_idx,
        "composite must come before destroy: {cmds:#?}"
    );
}

/// Binding an id that was never created (or was already destroyed)
/// must return an error without touching the bind stack.
#[test]
fn bind_unknown_id_errors() {
    let mut b = RecordingBackend::new(480, 272);
    let err = b
        .bind_render_target(RenderTargetId(999))
        .expect_err("bind should error");
    assert!(
        format!("{err:?}").contains("unknown id"),
        "unexpected error: {err:?}"
    );
    assert_eq!(b.render_target_bind_depth(), 0);
}

/// Destroying an id twice errors on the second call.
#[test]
fn double_destroy_errors() {
    let mut b = RecordingBackend::new(480, 272);
    let id = b.create_render_target(16, 16).expect("create");
    b.destroy_render_target(id).expect("destroy 1");
    let err = b.destroy_render_target(id).expect_err("destroy 2");
    assert!(format!("{err:?}").contains("unknown id"));
}

/// After a target is destroyed, its id is no longer bindable.
#[test]
fn bind_after_destroy_errors() {
    let mut b = RecordingBackend::new(480, 272);
    let id = b.create_render_target(16, 16).expect("create");
    b.destroy_render_target(id).expect("destroy");
    let err = b
        .bind_render_target(id)
        .expect_err("bind after destroy should error");
    assert!(format!("{err:?}").contains("unknown id"));
}

/// Popping an empty bind stack errors.
#[test]
fn unbind_underflow_errors() {
    let mut b = RecordingBackend::new(480, 272);
    let err = b.unbind_render_target().expect_err("underflow");
    assert!(
        format!("{err:?}").contains("underflow"),
        "unexpected error: {err:?}"
    );
}

/// Compositing an unknown id errors without recording anything.
#[test]
fn composite_unknown_id_errors() {
    let mut b = RecordingBackend::new(480, 272);
    let err = b
        .composite_render_target(RenderTargetId(42), 0, 0, 1, 1, BlendMode::Normal, 1.0)
        .expect_err("composite should error");
    assert!(format!("{err:?}").contains("unknown id"));
    // No CompositeRenderTarget command was recorded.
    assert!(
        !b.commands()
            .iter()
            .any(|c| matches!(c, DrawCommand::CompositeRenderTarget { .. })),
    );
}

/// `read_render_target` into a caller-supplied buffer returns zeros
/// (the recording backend never stores pixel data).
#[test]
fn read_render_target_returns_zeros() {
    let mut b = RecordingBackend::new(480, 272);
    let id = b.create_render_target(4, 4).expect("create");
    let mut dst = [0xFFu8; 4 * 4 * 4];
    b.read_render_target(id, &mut dst).expect("read");
    assert!(dst.iter().all(|&x| x == 0));
}

/// Capability probes are stable, idempotent, and do not record any
/// commands.
#[test]
fn capability_probes_are_stable() {
    let b = RecordingBackend::new(480, 272);
    assert!(b.supports_render_targets());
    assert!(b.supports_render_target_readback());
    assert!(b.supports_render_targets());
    assert_eq!(b.commands().len(), 0);
}

/// Two independent backends have independent render-target pools.
#[test]
fn pool_isolation_between_backends() {
    let mut a = RecordingBackend::new(480, 272);
    let mut c = RecordingBackend::new(480, 272);

    // Diverge histories: A creates three targets and destroys the
    // middle; C creates a single target and leaves it bound.
    let a0 = a.create_render_target(32, 32).expect("a0");
    let a1 = a.create_render_target(32, 32).expect("a1");
    let a2 = a.create_render_target(32, 32).expect("a2");
    a.destroy_render_target(a1).expect("destroy a1");
    a.bind_render_target(a0).expect("bind a0");
    a.unbind_render_target().expect("unbind a0");

    let c0 = c.create_render_target(128, 128).expect("c0");
    c.bind_render_target(c0).expect("bind c0");

    // If the pools were shared, at least one of these would be wrong:
    assert_eq!(a.live_render_target_count(), 2, "a: {:#?}", a.commands());
    assert_eq!(c.live_render_target_count(), 1, "c: {:#?}", c.commands());
    assert_eq!(a.render_target_bind_depth(), 0);
    assert_eq!(c.render_target_bind_depth(), 1);
    assert_eq!(c.currently_bound_render_target(), Some(c0));

    // Handles that only exist in A's pool do not validate in C's.
    // `a2` is `RenderTargetId(3)` on A; C only allocated one target so
    // id 3 is out of range for its pool.
    assert_eq!(a2, RenderTargetId(3));
    let err = c
        .bind_render_target(a2)
        .expect_err("foreign handle should not validate");
    assert!(format!("{err:?}").contains("unknown id"));
    // a2 still valid on its own backend.
    a.bind_render_target(a2).expect("a2 still valid");
    a.unbind_render_target().expect("unbind a2");
}

/// Composite handles opacity at the boundary values.
#[test]
fn composite_accepts_boundary_opacities() {
    let mut b = RecordingBackend::new(480, 272);
    let id = b.create_render_target(16, 16).expect("create");
    b.composite_render_target(id, 0, 0, 16, 16, BlendMode::Normal, 0.0)
        .expect("opacity 0");
    b.composite_render_target(id, 0, 0, 16, 16, BlendMode::Normal, 1.0)
        .expect("opacity 1");
}

/// All 16 blend modes round-trip through the recorder unchanged.
#[test]
fn all_blend_modes_recorded_distinctly() {
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    let mut b = RecordingBackend::new(480, 272);
    let id = b.create_render_target(8, 8).expect("create");
    for mode in modes {
        b.composite_render_target(id, 0, 0, 8, 8, mode, 1.0)
            .expect("composite");
    }

    let recorded: Vec<BlendMode> = b
        .commands()
        .iter()
        .filter_map(|c| match c {
            DrawCommand::CompositeRenderTarget { blend, .. } => Some(*blend),
            _ => None,
        })
        .collect();
    assert_eq!(recorded, modes);
}
