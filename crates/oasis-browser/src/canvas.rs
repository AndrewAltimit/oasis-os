//! Canvas 2D rendering context for `<canvas>` elements.
//!
//! Drawing commands are recorded into a [`CanvasState`] by JavaScript
//! (via the `__oasis_canvas_*` bindings in `js_dom/canvas.rs`) and then
//! replayed at paint time through the SDI backend.
//!
//! Supported API: `fillRect`, `strokeRect`, `clearRect`, `fillText`,
//! `beginPath`, `moveTo`, `lineTo`, `bezierCurveTo`, `quadraticCurveTo`,
//! `closePath`, `fill`, `stroke`, `arc`, `save`, `restore`, plus
//! properties `fillStyle`, `strokeStyle`, `lineWidth`, `font`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use oasis_types::backend::Color;

use crate::html::dom::NodeId;

/// Shared canvas state map accessible by both layout and JS bindings.
pub type SharedCanvasMap = Rc<RefCell<HashMap<NodeId, Rc<RefCell<CanvasState>>>>>;

/// A recorded drawing command for deferred canvas rendering.
#[derive(Debug, Clone)]
pub enum CanvasCommand {
    FillRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
    },
    StrokeRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        line_width: f32,
    },
    ClearRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    FillText {
        text: String,
        x: f32,
        y: f32,
        color: Color,
        font_size: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: Color,
        line_width: f32,
    },
    Arc {
        cx: f32,
        cy: f32,
        r: f32,
        color: Color,
        fill: bool,
    },
    /// Filled polygon path.
    FillPath {
        points: Vec<(f32, f32)>,
        color: Color,
    },
    /// Stroked polygon path.
    StrokePath {
        points: Vec<(f32, f32)>,
        color: Color,
        line_width: f32,
    },
}

impl CanvasCommand {
    /// Path points the command holds (0 for non-path commands).
    fn point_count(&self) -> usize {
        match self {
            Self::FillPath { points, .. } | Self::StrokePath { points, .. } => points.len(),
            _ => 0,
        }
    }
}

/// Most commands one canvas keeps (see [`CanvasState::push_command`]).
pub const MAX_CANVAS_COMMANDS: usize = 16_384;
/// Most path points one canvas's commands hold in total.
pub const MAX_CANVAS_POINTS: usize = 1 << 20;
/// Deepest `save()` stack kept.
const MAX_SAVE_DEPTH: usize = 1024;

/// Saved canvas drawing state for save()/restore().
#[derive(Debug, Clone)]
struct CanvasSavedState {
    fill_color: Color,
    stroke_color: Color,
    line_width: f32,
    font_size: f32,
}

/// State for a single `<canvas>` element's 2D rendering context.
#[derive(Debug, Clone)]
pub struct CanvasState {
    /// Intrinsic width of the canvas (from `width` attribute).
    pub width: u32,
    /// Intrinsic height of the canvas (from `height` attribute).
    pub height: u32,
    /// Recorded drawing commands.
    pub commands: Vec<CanvasCommand>,
    /// Current fill color.
    pub fill_color: Color,
    /// Current stroke color.
    pub stroke_color: Color,
    /// Current line width.
    pub line_width: f32,
    /// Current font size.
    pub font_size: f32,
    /// Current path being built.
    pub current_path: Vec<(f32, f32)>,
    /// Start of the current sub-path (for closePath).
    pub path_start: Option<(f32, f32)>,
    /// State save stack.
    state_stack: Vec<CanvasSavedState>,
    /// Path points held by `commands` (pushed via [`Self::push_command`]).
    held_points: usize,
}

impl CanvasState {
    /// Create a new canvas state with the given intrinsic dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            commands: Vec::new(),
            fill_color: Color::BLACK,
            stroke_color: Color::BLACK,
            line_width: 1.0,
            font_size: 10.0,
            current_path: Vec::new(),
            path_start: None,
            state_stack: Vec::new(),
            held_points: 0,
        }
    }

    /// Record a drawing command.
    ///
    /// The command list is replayed on every paint and lives outside the
    /// script heap limit, so it is bounded: a `clearRect` covering the
    /// whole canvas drops everything before it (the standard animation
    /// loop, `clearRect` + redraw per frame, used to grow it forever),
    /// and past [`MAX_CANVAS_COMMANDS`] commands or
    /// [`MAX_CANVAS_POINTS`] path points the oldest half is dropped.
    pub fn push_command(&mut self, cmd: CanvasCommand) {
        if let CanvasCommand::ClearRect { x, y, w, h } = cmd
            && x <= 0.0
            && y <= 0.0
            && x + w >= self.width as f32
            && y + h >= self.height as f32
        {
            self.commands.clear();
            self.held_points = 0;
            return;
        }
        self.held_points += cmd.point_count();
        self.commands.push(cmd);
        if self.commands.len() > MAX_CANVAS_COMMANDS || self.held_points > MAX_CANVAS_POINTS {
            let keep_from = self.commands.len() / 2;
            let dropped: usize = self
                .commands
                .drain(..keep_from)
                .map(|c| c.point_count())
                .sum();
            self.held_points -= dropped;
        }
    }

    /// Append a point to the current path, up to the SVG path point
    /// budget (65 536; further points are dropped).
    pub fn push_path_point(&mut self, point: (f32, f32)) {
        if self.current_path.len() < crate::svg::MAX_PATH_POINTS {
            self.current_path.push(point);
        }
    }

    /// Save the current drawing state (up to [`MAX_SAVE_DEPTH`] deep;
    /// deeper saves are ignored).
    pub fn save(&mut self) {
        if self.state_stack.len() >= MAX_SAVE_DEPTH {
            return;
        }
        self.state_stack.push(CanvasSavedState {
            fill_color: self.fill_color,
            stroke_color: self.stroke_color,
            line_width: self.line_width,
            font_size: self.font_size,
        });
    }

    /// Restore the most recently saved drawing state.
    pub fn restore(&mut self) {
        if let Some(saved) = self.state_stack.pop() {
            self.fill_color = saved.fill_color;
            self.stroke_color = saved.stroke_color;
            self.line_width = saved.line_width;
            self.font_size = saved.font_size;
        }
    }
}

/// Paint canvas commands to the backend.
///
/// Scales from canvas coordinate space to layout pixel space using
/// the ratio of layout dimensions (`w`, `h`) to canvas intrinsic
/// dimensions (`state.width`, `state.height`).
pub fn paint_canvas(
    state: &CanvasState,
    backend: &mut dyn oasis_types::backend::SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> oasis_types::error::Result<()> {
    if state.width == 0 || state.height == 0 {
        return Ok(());
    }

    let sx = w as f32 / state.width as f32;
    let sy = h as f32 / state.height as f32;

    for cmd in &state.commands {
        match cmd {
            CanvasCommand::FillRect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
                color,
            } => {
                backend.fill_rect(
                    x + (*cx * sx) as i32,
                    y + (*cy * sy) as i32,
                    (*cw * sx) as u32,
                    (*ch * sy) as u32,
                    *color,
                )?;
            },
            CanvasCommand::ClearRect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
            } => {
                backend.fill_rect(
                    x + (*cx * sx) as i32,
                    y + (*cy * sy) as i32,
                    (*cw * sx) as u32,
                    (*ch * sy) as u32,
                    Color::rgba(0, 0, 0, 0),
                )?;
            },
            CanvasCommand::FillText {
                text,
                x: cx,
                y: cy,
                color,
                font_size,
            } => {
                backend.draw_text(
                    text,
                    x + (*cx * sx) as i32,
                    y + (*cy * sy) as i32,
                    (*font_size * sy) as u16,
                    *color,
                )?;
            },
            CanvasCommand::StrokeRect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
                color,
                line_width,
            } => {
                let lw = (*line_width * sx).max(1.0) as u32;
                let rx = x + (*cx * sx) as i32;
                let ry = y + (*cy * sy) as i32;
                let rw = (*cw * sx) as u32;
                let rh = (*ch * sy) as u32;
                // Top edge
                backend.fill_rect(rx, ry, rw, lw, *color)?;
                // Bottom edge
                backend.fill_rect(rx, ry + rh as i32 - lw as i32, rw, lw, *color)?;
                // Left edge (between top and bottom to avoid corner overlap)
                backend.fill_rect(rx, ry + lw as i32, lw, rh.saturating_sub(lw * 2), *color)?;
                // Right edge (between top and bottom to avoid corner overlap)
                backend.fill_rect(
                    rx + rw as i32 - lw as i32,
                    ry + lw as i32,
                    lw,
                    rh.saturating_sub(lw * 2),
                    *color,
                )?;
            },
            CanvasCommand::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                line_width,
            } => {
                let lw = (*line_width * sx).max(1.0) as u32;
                let px1 = x + (*x1 * sx) as i32;
                let py1 = y + (*y1 * sy) as i32;
                let px2 = x + (*x2 * sx) as i32;
                let py2 = y + (*y2 * sy) as i32;
                if (py2 - py1).abs() <= (px2 - px1).abs() {
                    // More horizontal
                    let (lx, rx) = if px1 < px2 { (px1, px2) } else { (px2, px1) };
                    let mid_y = (py1 + py2) / 2;
                    backend.fill_rect(lx, mid_y, (rx - lx) as u32, lw, *color)?;
                } else {
                    // More vertical
                    let (ty, by) = if py1 < py2 { (py1, py2) } else { (py2, py1) };
                    let mid_x = (px1 + px2) / 2;
                    backend.fill_rect(mid_x, ty, lw, (by - ty) as u32, *color)?;
                }
            },
            CanvasCommand::Arc {
                cx,
                cy,
                r,
                color,
                fill,
            } => {
                let rx = x + ((*cx - *r) * sx) as i32;
                let ry = y + ((*cy - *r) * sy) as i32;
                let d = (*r * 2.0 * sx) as u32;
                let radius = (*r * sx) as u16;
                if *fill {
                    backend.fill_rounded_rect(rx, ry, d, d, radius, *color)?;
                } else {
                    // Stroke: draw a ring outline.
                    backend.stroke_rounded_rect(rx, ry, d, d, radius, 1, *color)?;
                }
            },
            CanvasCommand::FillPath { points, color } => {
                if points.len() >= 3 {
                    let screen_pts: Vec<(i32, i32)> = points
                        .iter()
                        .map(|&(px, py)| (x + (px * sx) as i32, y + (py * sy) as i32))
                        .collect();
                    backend.fill_polygon(&screen_pts, *color)?;
                }
            },
            CanvasCommand::StrokePath {
                points,
                color,
                line_width,
            } => {
                if points.len() >= 2 {
                    let screen_pts: Vec<(i32, i32)> = points
                        .iter()
                        .map(|&(px, py)| (x + (px * sx) as i32, y + (py * sy) as i32))
                        .collect();
                    let sw = (*line_width * sx).max(1.0) as u16;
                    backend.stroke_polyline(&screen_pts, sw, *color)?;
                }
            },
        }
    }
    Ok(())
}

/// Walk a layout tree and collect all `ReplacedContent::Canvas` states
/// into the shared map, keyed by DOM `NodeId`.
pub fn collect_canvas_states(
    layout_box: &crate::layout::box_model::LayoutBox,
    map: &SharedCanvasMap,
) {
    if let crate::layout::box_model::BoxType::Replaced(
        crate::layout::box_model::ReplacedContent::Canvas { ref state },
    ) = layout_box.box_type
        && let Some(nid) = layout_box.node
    {
        map.borrow_mut().insert(nid, Rc::clone(state));
    }
    for child in &layout_box.children {
        collect_canvas_states(child, map);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_state_defaults() {
        let cs = CanvasState::new(300, 150);
        assert_eq!(cs.width, 300);
        assert_eq!(cs.height, 150);
        assert!(cs.commands.is_empty());
        assert_eq!(cs.fill_color, Color::BLACK);
        assert_eq!(cs.stroke_color, Color::BLACK);
        assert!((cs.line_width - 1.0).abs() < f32::EPSILON);
        assert!((cs.font_size - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn canvas_state_records_commands() {
        let mut cs = CanvasState::new(100, 100);
        cs.commands.push(CanvasCommand::FillRect {
            x: 10.0,
            y: 20.0,
            w: 50.0,
            h: 30.0,
            color: Color::rgb(255, 0, 0),
        });
        cs.commands.push(CanvasCommand::ClearRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        });
        assert_eq!(cs.commands.len(), 2);
    }

    #[test]
    fn paint_canvas_zero_size_is_noop() {
        let cs = CanvasState::new(0, 0);
        let mut backend = crate::test_utils::MockBackend::new();
        let result = paint_canvas(&cs, &mut backend, 0, 0, 0, 0);
        assert!(result.is_ok());
    }

    /// Regression: the command list was never cleared, so the standard
    /// animation loop (`clearRect` + redraw each frame) grew it, and the
    /// per-paint replay, forever.
    #[test]
    fn full_canvas_clear_drops_earlier_commands() {
        let mut cs = CanvasState::new(64, 32);
        for i in 0..10_000 {
            cs.push_command(CanvasCommand::ClearRect {
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 32.0,
            });
            cs.push_command(CanvasCommand::FillRect {
                x: i as f32 % 60.0,
                y: 0.0,
                w: 4.0,
                h: 4.0,
                color: Color::BLACK,
            });
        }
        assert_eq!(cs.commands.len(), 1);
        // A partial clear is kept (it is not a reset).
        cs.push_command(CanvasCommand::ClearRect {
            x: 1.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        });
        assert_eq!(cs.commands.len(), 2);
    }

    /// Without clears, commands, path points and save depth stay bounded.
    #[test]
    fn canvas_state_growth_is_bounded() {
        let mut cs = CanvasState::new(10, 10);
        for _ in 0..(MAX_CANVAS_COMMANDS * 3) {
            cs.push_command(CanvasCommand::FillRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                color: Color::BLACK,
            });
        }
        assert!(cs.commands.len() <= MAX_CANVAS_COMMANDS);
        for i in 0..200_000 {
            cs.push_path_point((i as f32, 0.0));
        }
        assert_eq!(cs.current_path.len(), crate::svg::MAX_PATH_POINTS);
        for _ in 0..40 {
            let points = cs.current_path.clone();
            cs.push_command(CanvasCommand::FillPath {
                points,
                color: Color::BLACK,
            });
        }
        assert!(cs.held_points <= MAX_CANVAS_POINTS);
        for _ in 0..5000 {
            cs.save();
        }
        assert_eq!(cs.state_stack.len(), MAX_SAVE_DEPTH);
    }
}
