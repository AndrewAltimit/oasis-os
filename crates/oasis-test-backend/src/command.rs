//! [`DrawCommand`]: the recording backends' draw-call log entry.

use oasis_types::backend::{BlendMode, Color, GradientStyle, RenderTargetId, TextureId};

/// One recorded draw call, as logged by [`RecordingBackend`] and
/// [`ClipAuditBackend`].
///
/// Each variant captures the parameters of one `SdiBackend` method so tests
/// can assert on the exact sequence of calls code under test makes. It is a
/// test log format only; no renderer consumes it.
///
/// [`RecordingBackend`]: crate::RecordingBackend
/// [`ClipAuditBackend`]: crate::ClipAuditBackend
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    FillRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    },
    FillRoundedRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    },
    StrokeRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        stroke_width: u16,
        color: Color,
    },
    DrawLine {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    },
    FillCircle {
        cx: i32,
        cy: i32,
        radius: u16,
        color: Color,
    },
    FillTriangle {
        points: [(i32, i32); 3],
        color: Color,
    },
    Gradient {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: GradientStyle,
    },
    DrawText {
        text: String,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    },
    Blit {
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    BlitSub {
        tex: TextureId,
        src: (u32, u32, u32, u32),
        dst: (i32, i32, u32, u32),
    },
    BlitTinted {
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        tint: Color,
    },
    /// `push_clip_rect` (the arguments as passed: `RecordingBackend` logs
    /// them untranslated, `ClipAuditBackend` translated).
    PushClip {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    /// `pop_clip_rect`.
    PopClip,
    /// `set_clip_rect`: absolute, replaces the clip without touching the
    /// clip stack (see the `SdiCore` drawing-state contract).
    SetClip {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    /// `reset_clip_rect`: removes the clip.
    ResetClip,
    PushTranslate {
        dx: i32,
        dy: i32,
    },
    PopTranslate,
    FillPolygon {
        points: Vec<(i32, i32)>,
        color: Color,
    },
    FillArc {
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    },
    StrokeArc {
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        width: u16,
        color: Color,
    },
    StrokeLineDashed {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
        dash: u16,
        gap: u16,
    },
    /// Allocate an offscreen render target.
    CreateRenderTarget {
        id: RenderTargetId,
        w: u32,
        h: u32,
    },
    /// Push a render target onto the bind stack.
    BindRenderTarget {
        id: RenderTargetId,
    },
    /// Pop the most recent `BindRenderTarget`.
    UnbindRenderTarget,
    /// Composite a render target back into the currently bound surface.
    CompositeRenderTarget {
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        blend: BlendMode,
        opacity: f32,
    },
    /// Composite a premultiplied-alpha render target back into the
    /// currently bound surface with source-over blending.
    CompositeRenderTargetPremultiplied {
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    },
    /// Release a render target.
    DestroyRenderTarget {
        id: RenderTargetId,
    },
}
