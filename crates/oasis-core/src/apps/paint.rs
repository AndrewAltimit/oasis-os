//! Paint/Drawing application for OASIS OS.
//!
//! A pixel-art drawing app with multi-layer canvas, multiple drawing
//! tools (pencil, line, rectangle, circle, eraser, flood fill),
//! a 16-color palette, undo/redo, and grid overlay. The canvas is
//! rendered scaled up to fit the content area.

use std::any::Any;

use crate::active_theme::ActiveTheme;
use crate::backend::{Color, SdiBackend};
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;

use super::app_trait::{App, ContentState};
use super::file_manager::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use super::runner::AppAction;

// ---------------------------------------------------------------
// Constants
// ---------------------------------------------------------------

/// Default canvas width in pixels.
const CANVAS_W: u32 = 64;
/// Default canvas height in pixels.
const CANVAS_H: u32 = 48;
/// Maximum undo history depth.
const MAX_UNDO: usize = 20;

/// Standard 16-color palette.
pub fn palette() -> [Color; 16] {
    [
        Color::rgb(0, 0, 0),       // Black
        Color::rgb(255, 255, 255), // White
        Color::rgb(255, 0, 0),     // Red
        Color::rgb(0, 255, 0),     // Green
        Color::rgb(0, 0, 255),     // Blue
        Color::rgb(255, 255, 0),   // Yellow
        Color::rgb(255, 0, 255),   // Magenta
        Color::rgb(0, 255, 255),   // Cyan
        Color::rgb(128, 128, 128), // Gray
        Color::rgb(128, 0, 0),     // Dark Red
        Color::rgb(0, 128, 0),     // Dark Green
        Color::rgb(0, 0, 128),     // Dark Blue
        Color::rgb(128, 128, 0),   // Olive
        Color::rgb(128, 0, 128),   // Purple
        Color::rgb(0, 128, 128),   // Teal
        Color::rgb(255, 128, 0),   // Orange
    ]
}

/// Palette color names (for display).
const PALETTE_NAMES: [&str; 16] = [
    "Black", "White", "Red", "Green", "Blue", "Yellow", "Magenta", "Cyan", "Gray", "DkRed",
    "DkGreen", "DkBlue", "Olive", "Purple", "Teal", "Orange",
];

// ---------------------------------------------------------------
// Tool
// ---------------------------------------------------------------

/// Drawing tool selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pencil,
    Line,
    Rectangle,
    FilledRectangle,
    Circle,
    FilledCircle,
    Eraser,
    Fill,
}

impl Tool {
    /// All tool variants in cycle order.
    const ALL: [Tool; 8] = [
        Tool::Pencil,
        Tool::Line,
        Tool::Rectangle,
        Tool::FilledRectangle,
        Tool::Circle,
        Tool::FilledCircle,
        Tool::Eraser,
        Tool::Fill,
    ];

    /// Advance to the next tool in cycle order.
    pub fn next(self) -> Tool {
        let idx = Tool::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Tool::ALL[(idx + 1) % Tool::ALL.len()]
    }

    /// Display name for the tool.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Pencil => "Pencil",
            Tool::Line => "Line",
            Tool::Rectangle => "Rect",
            Tool::FilledRectangle => "FillRect",
            Tool::Circle => "Circle",
            Tool::FilledCircle => "FillCircle",
            Tool::Eraser => "Eraser",
            Tool::Fill => "Fill",
        }
    }
}

// ---------------------------------------------------------------
// Layer
// ---------------------------------------------------------------

/// A single layer in the canvas stack.
#[derive(Debug, Clone)]
pub struct Layer {
    name: String,
    pixels: Vec<Color>,
    visible: bool,
    opacity: u8,
}

impl Layer {
    /// Create a new layer filled with the given color.
    fn new(name: &str, w: u32, h: u32, fill: Color) -> Self {
        Self {
            name: name.to_string(),
            pixels: vec![fill; (w * h) as usize],
            visible: true,
            opacity: 255,
        }
    }

    /// Create a new transparent layer.
    fn new_transparent(name: &str, w: u32, h: u32) -> Self {
        Self::new(name, w, h, Color::rgba(0, 0, 0, 0))
    }
}

// ---------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------

/// Multi-layer pixel canvas.
#[derive(Debug, Clone)]
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<Color>,
    layers: Vec<Layer>,
    active_layer: usize,
}

impl Canvas {
    /// Create a new canvas with one white background layer.
    pub fn new(width: u32, height: u32) -> Self {
        let bg = Layer::new("Background", width, height, Color::rgb(255, 255, 255));
        let pixels = bg.pixels.clone();
        Self {
            width,
            height,
            pixels,
            layers: vec![bg],
            active_layer: 0,
        }
    }

    /// Canvas width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Canvas height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the color of a pixel from the flattened view.
    pub fn get_pixel(&self, x: u32, y: u32) -> Color {
        if x >= self.width || y >= self.height {
            return Color::rgba(0, 0, 0, 0);
        }
        self.pixels[(y * self.width + x) as usize]
    }

    /// Set a pixel on the active layer.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        if let Some(layer) = self.layers.get_mut(self.active_layer) {
            layer.pixels[(y * self.width + x) as usize] = color;
        }
        self.pixels = self.flatten();
    }

    /// Fill the active layer with a color.
    pub fn fill(&mut self, color: Color) {
        if let Some(layer) = self.layers.get_mut(self.active_layer) {
            for px in &mut layer.pixels {
                *px = color;
            }
        }
        self.pixels = self.flatten();
    }

    /// Clear the active layer to transparent.
    pub fn clear(&mut self) {
        self.fill(Color::rgba(0, 0, 0, 0));
    }

    /// Flatten all visible layers (bottom to top) with alpha blending.
    pub fn flatten(&self) -> Vec<Color> {
        let size = (self.width * self.height) as usize;
        let mut result = vec![Color::rgba(0, 0, 0, 0); size];

        for layer in &self.layers {
            if !layer.visible {
                continue;
            }
            let layer_alpha = layer.opacity;
            for (dst, src) in result.iter_mut().zip(&layer.pixels) {
                *dst = alpha_blend(*src, *dst, layer_alpha);
            }
        }
        result
    }

    /// Add a new transparent layer. Returns its index.
    pub fn add_layer(&mut self, name: &str) -> usize {
        let layer = Layer::new_transparent(name, self.width, self.height);
        self.layers.push(layer);
        let idx = self.layers.len() - 1;
        self.pixels = self.flatten();
        idx
    }

    /// Set the active layer by index.
    pub fn set_active_layer(&mut self, index: usize) {
        if index < self.layers.len() {
            self.active_layer = index;
        }
    }

    /// Get the active layer index.
    pub fn active_layer(&self) -> usize {
        self.active_layer
    }

    /// Toggle visibility of a layer by index.
    pub fn toggle_layer_visibility(&mut self, index: usize) {
        if let Some(layer) = self.layers.get_mut(index) {
            layer.visible = !layer.visible;
        }
        self.pixels = self.flatten();
    }

    /// Number of layers.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Name of a layer.
    pub fn layer_name(&self, index: usize) -> &str {
        self.layers
            .get(index)
            .map(|l| l.name.as_str())
            .unwrap_or("")
    }

    /// Get a mutable reference to the active layer's pixels.
    pub fn active_pixels_mut(&mut self) -> Option<&mut Vec<Color>> {
        self.layers
            .get_mut(self.active_layer)
            .map(|l| &mut l.pixels)
    }

    /// Snapshot the active layer's pixels (for undo).
    pub fn snapshot_active(&self) -> Option<Vec<Color>> {
        self.layers.get(self.active_layer).map(|l| l.pixels.clone())
    }

    /// Restore the active layer's pixels from a snapshot.
    pub fn restore_active(&mut self, snapshot: &[Color]) {
        if let Some(layer) = self.layers.get_mut(self.active_layer) {
            let len = layer.pixels.len().min(snapshot.len());
            layer.pixels[..len].copy_from_slice(&snapshot[..len]);
        }
        self.pixels = self.flatten();
    }

    /// Re-flatten after batch pixel operations on the active layer.
    pub fn refresh_flat(&mut self) {
        self.pixels = self.flatten();
    }
}

/// Alpha-blend `src` over `dst`, with an extra layer opacity.
fn alpha_blend(src: Color, dst: Color, layer_alpha: u8) -> Color {
    let sa = (src.a as u32 * layer_alpha as u32) / 255;
    if sa == 0 {
        return dst;
    }
    if sa == 255 && dst.a == 0 {
        return Color::rgba(src.r, src.g, src.b, sa as u8);
    }
    let da = dst.a as u32;
    let out_a = sa + da * (255 - sa) / 255;
    if out_a == 0 {
        return Color::rgba(0, 0, 0, 0);
    }
    let r = (src.r as u32 * sa + dst.r as u32 * da * (255 - sa) / 255) / out_a;
    let g = (src.g as u32 * sa + dst.g as u32 * da * (255 - sa) / 255) / out_a;
    let b = (src.b as u32 * sa + dst.b as u32 * da * (255 - sa) / 255) / out_a;
    Color::rgba(
        r.min(255) as u8,
        g.min(255) as u8,
        b.min(255) as u8,
        out_a.min(255) as u8,
    )
}

// ---------------------------------------------------------------
// Drawing primitives (pure functions on pixel buffers)
// ---------------------------------------------------------------

/// Set a single pixel with bounds checking.
pub fn draw_pixel(pixels: &mut [Color], w: u32, x: i32, y: i32, color: Color) {
    if x < 0 || y < 0 {
        return;
    }
    let ux = x as u32;
    let uy = y as u32;
    let h = pixels.len() as u32 / w.max(1);
    if ux < w && uy < h {
        pixels[(uy * w + ux) as usize] = color;
    }
}

/// Draw a line using Bresenham's algorithm.
#[allow(clippy::too_many_arguments)]
pub fn draw_line(
    pixels: &mut [Color],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: Color,
    brush_size: u32,
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx: i32 = if x0 < x1 { 1 } else { -1 };
    let sy: i32 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut cx = x0;
    let mut cy = y0;

    loop {
        if brush_size <= 1 {
            draw_pixel(pixels, w, cx, cy, color);
        } else {
            draw_brush(pixels, w, h, cx, cy, brush_size, color);
        }
        if cx == x1 && cy == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if cx == x1 {
                break;
            }
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            if cy == y1 {
                break;
            }
            err += dx;
            cy += sy;
        }
    }
}

/// Draw an outline rectangle.
#[allow(clippy::too_many_arguments)]
pub fn draw_rect(
    pixels: &mut [Color],
    w: u32,
    h: u32,
    x: i32,
    y: i32,
    rw: u32,
    rh: u32,
    color: Color,
) {
    if rw == 0 || rh == 0 {
        return;
    }
    let x2 = x + rw as i32 - 1;
    let y2 = y + rh as i32 - 1;
    // Top and bottom edges.
    for px in x..=x2 {
        draw_pixel(pixels, w, px, y, color);
        draw_pixel(pixels, w, px, y2, color);
    }
    // Left and right edges (skip corners already drawn).
    for py in (y + 1)..y2 {
        draw_pixel(pixels, w, x, py, color);
        draw_pixel(pixels, w, x2, py, color);
    }
    let _ = h; // bounds checked inside draw_pixel
}

/// Draw a filled rectangle.
#[allow(clippy::too_many_arguments)]
pub fn draw_filled_rect(
    pixels: &mut [Color],
    w: u32,
    h: u32,
    x: i32,
    y: i32,
    rw: u32,
    rh: u32,
    color: Color,
) {
    for dy in 0..rh as i32 {
        for dx in 0..rw as i32 {
            draw_pixel(pixels, w, x + dx, y + dy, color);
        }
    }
    let _ = h;
}

/// Draw an outline circle using the midpoint algorithm.
pub fn draw_circle(pixels: &mut [Color], w: u32, h: u32, cx: i32, cy: i32, r: u32, color: Color) {
    if r == 0 {
        draw_pixel(pixels, w, cx, cy, color);
        return;
    }
    let _ = h;
    let r = r as i32;
    let mut x = r;
    let mut y: i32 = 0;
    let mut d = 1 - r;

    while x >= y {
        draw_pixel(pixels, w, cx + x, cy + y, color);
        draw_pixel(pixels, w, cx - x, cy + y, color);
        draw_pixel(pixels, w, cx + x, cy - y, color);
        draw_pixel(pixels, w, cx - x, cy - y, color);
        draw_pixel(pixels, w, cx + y, cy + x, color);
        draw_pixel(pixels, w, cx - y, cy + x, color);
        draw_pixel(pixels, w, cx + y, cy - x, color);
        draw_pixel(pixels, w, cx - y, cy - x, color);

        y += 1;
        if d <= 0 {
            d += 2 * y + 1;
        } else {
            x -= 1;
            d += 2 * (y - x) + 1;
        }
    }
}

/// Draw a filled circle.
pub fn draw_filled_circle(
    pixels: &mut [Color],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    r: u32,
    color: Color,
) {
    if r == 0 {
        draw_pixel(pixels, w, cx, cy, color);
        return;
    }
    let _ = h;
    let ri = r as i32;
    let r2 = ri * ri;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if dx * dx + dy * dy <= r2 {
                draw_pixel(pixels, w, cx + dx, cy + dy, color);
            }
        }
    }
}

/// Flood fill from a point using a stack-based algorithm.
pub fn flood_fill(pixels: &mut [Color], w: u32, h: u32, x: i32, y: i32, fill_color: Color) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return;
    }
    let idx = (y as u32 * w + x as u32) as usize;
    let target = pixels[idx];
    if target == fill_color {
        return;
    }

    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 {
            continue;
        }
        let si = (sy as u32 * w + sx as u32) as usize;
        if pixels[si] != target {
            continue;
        }
        pixels[si] = fill_color;
        stack.push((sx + 1, sy));
        stack.push((sx - 1, sy));
        stack.push((sx, sy + 1));
        stack.push((sx, sy - 1));
    }
}

/// Draw a square brush (size x size centered on point).
pub fn draw_brush(pixels: &mut [Color], w: u32, h: u32, x: i32, y: i32, size: u32, color: Color) {
    let half = size as i32 / 2;
    for dy in 0..size as i32 {
        for dx in 0..size as i32 {
            draw_pixel(pixels, w, x - half + dx, y - half + dy, color);
        }
    }
    let _ = h;
}

// ---------------------------------------------------------------
// UndoEntry
// ---------------------------------------------------------------

/// Encode pixel data as a 32-bit BMP file (BGRA, bottom-up).
fn encode_bmp(pixels: &[Color], w: u32, h: u32) -> Vec<u8> {
    let row_size = w * 4;
    let pixel_data_size = row_size * h;
    let file_size = 54 + pixel_data_size;
    let mut buf = Vec::with_capacity(file_size as usize);

    // BMP file header (14 bytes).
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(file_size).to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
    buf.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset

    // DIB header (BITMAPINFOHEADER, 40 bytes).
    buf.extend_from_slice(&40u32.to_le_bytes()); // header size
    buf.extend_from_slice(&(w as i32).to_le_bytes());
    buf.extend_from_slice(&(h as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // planes
    buf.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    buf.extend_from_slice(&0u32.to_le_bytes()); // compression (BI_RGB)
    buf.extend_from_slice(&pixel_data_size.to_le_bytes());
    buf.extend_from_slice(&2835u32.to_le_bytes()); // x pixels/meter
    buf.extend_from_slice(&2835u32.to_le_bytes()); // y pixels/meter
    buf.extend_from_slice(&0u32.to_le_bytes()); // colors used
    buf.extend_from_slice(&0u32.to_le_bytes()); // important colors

    // Pixel data (bottom-up, BGRA).
    for y in (0..h).rev() {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let c = if idx < pixels.len() {
                pixels[idx]
            } else {
                Color::rgba(0, 0, 0, 0)
            };
            buf.push(c.b);
            buf.push(c.g);
            buf.push(c.r);
            buf.push(c.a);
        }
    }
    buf
}

/// A snapshot of a layer's pixels for undo/redo.
#[derive(Debug, Clone)]
pub struct UndoEntry {
    layer: usize,
    snapshot: Vec<Color>,
}

// ---------------------------------------------------------------
// PaintApp
// ---------------------------------------------------------------

/// The Paint/Drawing application.
#[derive(Debug)]
pub struct PaintApp {
    content: ContentState,
    canvas: Canvas,
    tool: Tool,
    color: Color,
    palette_index: usize,
    brush_size: u32,
    cursor_x: i32,
    cursor_y: i32,
    drawing: bool,
    drag_start: Option<(i32, i32)>,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    show_grid: bool,
    display_lines: Vec<String>,
}

impl PaintApp {
    /// Create a new Paint application.
    pub fn new(path: &str) -> Self {
        let pal = palette();
        let mut app = Self {
            content: ContentState::new("Paint", path),
            canvas: Canvas::new(CANVAS_W, CANVAS_H),
            tool: Tool::Pencil,
            color: pal[0],
            palette_index: 0,
            brush_size: 1,
            cursor_x: (CANVAS_W / 2) as i32,
            cursor_y: (CANVAS_H / 2) as i32,
            drawing: false,
            drag_start: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            show_grid: false,
            display_lines: Vec::new(),
        };
        app.rebuild_display_lines();
        app
    }

    /// Push an undo snapshot for the current active layer.
    fn push_undo(&mut self) {
        if let Some(snapshot) = self.canvas.snapshot_active() {
            self.undo_stack.push(UndoEntry {
                layer: self.canvas.active_layer(),
                snapshot,
            });
            if self.undo_stack.len() > MAX_UNDO {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
        }
    }

    /// Undo the last operation.
    fn undo(&mut self) {
        if let Some(entry) = self.undo_stack.pop() {
            // Save current state for redo before restoring.
            let current_layer = self.canvas.active_layer();
            self.canvas.set_active_layer(entry.layer);
            if let Some(current_snap) = self.canvas.snapshot_active() {
                self.redo_stack.push(UndoEntry {
                    layer: entry.layer,
                    snapshot: current_snap,
                });
            }
            self.canvas.restore_active(&entry.snapshot);
            self.canvas.set_active_layer(current_layer);
        }
    }

    /// Redo the last undone operation.
    fn redo(&mut self) {
        if let Some(entry) = self.redo_stack.pop() {
            let current_layer = self.canvas.active_layer();
            self.canvas.set_active_layer(entry.layer);
            if let Some(current_snap) = self.canvas.snapshot_active() {
                self.undo_stack.push(UndoEntry {
                    layer: entry.layer,
                    snapshot: current_snap,
                });
            }
            self.canvas.restore_active(&entry.snapshot);
            self.canvas.set_active_layer(current_layer);
        }
    }

    /// Apply the current tool at the cursor position.
    fn apply_tool(&mut self) {
        let w = self.canvas.width();
        let h = self.canvas.height();
        let cx = self.cursor_x;
        let cy = self.cursor_y;
        let color = self.color;
        let brush = self.brush_size;

        match self.tool {
            Tool::Pencil => {
                if !self.drawing {
                    self.push_undo();
                    self.drawing = true;
                }
                if let Some(px) = self.canvas.active_pixels_mut() {
                    if brush <= 1 {
                        draw_pixel(px, w, cx, cy, color);
                    } else {
                        draw_brush(px, w, h, cx, cy, brush, color);
                    }
                }
                self.canvas.refresh_flat();
            },
            Tool::Eraser => {
                let erase = Color::rgba(0, 0, 0, 0);
                if !self.drawing {
                    self.push_undo();
                    self.drawing = true;
                }
                if let Some(px) = self.canvas.active_pixels_mut() {
                    if brush <= 1 {
                        draw_pixel(px, w, cx, cy, erase);
                    } else {
                        draw_brush(px, w, h, cx, cy, brush, erase);
                    }
                }
                self.canvas.refresh_flat();
            },
            Tool::Fill => {
                self.push_undo();
                if let Some(px) = self.canvas.active_pixels_mut() {
                    flood_fill(px, w, h, cx, cy, color);
                }
                self.canvas.refresh_flat();
                self.drawing = false;
            },
            Tool::Line
            | Tool::Rectangle
            | Tool::FilledRectangle
            | Tool::Circle
            | Tool::FilledCircle => {
                if self.drag_start.is_none() {
                    self.push_undo();
                    self.drag_start = Some((cx, cy));
                    self.drawing = true;
                } else {
                    self.finish_shape();
                }
            },
        }
    }

    /// Finish a shape tool (line/rect/circle) stroke.
    fn finish_shape(&mut self) {
        let Some((sx, sy)) = self.drag_start.take() else {
            return;
        };
        let w = self.canvas.width();
        let h = self.canvas.height();
        let ex = self.cursor_x;
        let ey = self.cursor_y;
        let color = self.color;
        let brush = self.brush_size;

        if let Some(px) = self.canvas.active_pixels_mut() {
            match self.tool {
                Tool::Line => {
                    draw_line(px, w, h, sx, sy, ex, ey, color, brush);
                },
                Tool::Rectangle => {
                    let rx = sx.min(ex);
                    let ry = sy.min(ey);
                    let rw = (sx - ex).unsigned_abs() + 1;
                    let rh = (sy - ey).unsigned_abs() + 1;
                    draw_rect(px, w, h, rx, ry, rw, rh, color);
                },
                Tool::FilledRectangle => {
                    let rx = sx.min(ex);
                    let ry = sy.min(ey);
                    let rw = (sx - ex).unsigned_abs() + 1;
                    let rh = (sy - ey).unsigned_abs() + 1;
                    draw_filled_rect(px, w, h, rx, ry, rw, rh, color);
                },
                Tool::Circle => {
                    let dx = (ex - sx) as f64;
                    let dy = (ey - sy) as f64;
                    let r = (dx * dx + dy * dy).sqrt() as u32;
                    draw_circle(px, w, h, sx, sy, r, color);
                },
                Tool::FilledCircle => {
                    let dx = (ex - sx) as f64;
                    let dy = (ey - sy) as f64;
                    let r = (dx * dx + dy * dy).sqrt() as u32;
                    draw_filled_circle(px, w, h, sx, sy, r, color);
                },
                _ => {},
            }
        }
        self.canvas.refresh_flat();
        self.drawing = false;
    }

    /// Stop the current drawing stroke.
    fn stop_drawing(&mut self) {
        self.drawing = false;
        self.drag_start = None;
    }

    /// Rebuild the text display lines.
    fn rebuild_display_lines(&mut self) {
        let pal_name = if self.palette_index < PALETTE_NAMES.len() {
            PALETTE_NAMES[self.palette_index]
        } else {
            "Custom"
        };
        let grid_str = if self.show_grid { "ON" } else { "OFF" };
        let layer_name = self.canvas.layer_name(self.canvas.active_layer());
        let layer_count = self.canvas.layer_count();
        let active_idx = self.canvas.active_layer() + 1;
        let drag_info = if let Some((sx, sy)) = self.drag_start {
            format!("  From: ({sx}, {sy})")
        } else {
            String::new()
        };

        self.display_lines = vec![
            format!("Paint - {}x{}", self.canvas.width(), self.canvas.height()),
            "\u{2500}".repeat(30),
            format!(
                "  Tool: {}  Color: {} ({})  Size: {}",
                self.tool.name(),
                "\u{2588}\u{2588}",
                pal_name,
                self.brush_size,
            ),
            format!("  [Grid: {grid_str}]"),
            format!(
                "  Cursor: ({}, {}){drag_info}",
                self.cursor_x, self.cursor_y,
            ),
            format!("  Layer: {layer_name} ({active_idx}/{layer_count})"),
            "\u{2500}".repeat(30),
            format!(
                "  Undo: {}  Redo: {}",
                self.undo_stack.len(),
                self.redo_stack.len(),
            ),
        ];
        self.content.lines = self.display_lines.clone();
    }

    /// Move the cursor, clamping to canvas bounds.
    fn move_cursor(&mut self, dx: i32, dy: i32) {
        let new_x = self.cursor_x + dx;
        let new_y = self.cursor_y + dy;
        self.cursor_x = new_x.clamp(0, self.canvas.width() as i32 - 1);
        self.cursor_y = new_y.clamp(0, self.canvas.height() as i32 - 1);
    }

    /// Cycle to the next palette color.
    fn cycle_color(&mut self) {
        let pal = palette();
        self.palette_index = (self.palette_index + 1) % pal.len();
        self.color = pal[self.palette_index];
    }

    /// Cycle brush size 1-5.
    pub fn cycle_brush_size(&mut self) {
        self.brush_size = (self.brush_size % 5) + 1;
    }

    /// Save the canvas to VFS as a BMP file.
    pub fn save_to_vfs(&self, vfs: &mut dyn Vfs) -> bool {
        let w = self.canvas.width();
        let h = self.canvas.height();
        let flat = self.canvas.flatten();
        let bmp = encode_bmp(&flat, w, h);
        let save_dir = "/home/user/pictures";
        let _ = vfs.mkdir(save_dir);
        let path = format!("{save_dir}/paint_{w}x{h}.bmp");
        vfs.write(&path, &bmp).is_ok()
    }

    /// Create a new canvas with the given dimensions.
    pub fn new_canvas(&mut self, width: u32, height: u32) {
        let w = width.clamp(8, 256);
        let h = height.clamp(8, 256);
        self.canvas = Canvas::new(w, h);
        self.cursor_x = (w / 2) as i32;
        self.cursor_y = (h / 2) as i32;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.drawing = false;
        self.drag_start = None;
        self.rebuild_display_lines();
    }
}

impl App for PaintApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Up => {
                self.move_cursor(0, -1);
                if self.drawing && (self.tool == Tool::Pencil || self.tool == Tool::Eraser) {
                    self.apply_tool();
                }
            },
            Button::Down => {
                self.move_cursor(0, 1);
                if self.drawing && (self.tool == Tool::Pencil || self.tool == Tool::Eraser) {
                    self.apply_tool();
                }
            },
            Button::Left => {
                self.move_cursor(-1, 0);
                if self.drawing && (self.tool == Tool::Pencil || self.tool == Tool::Eraser) {
                    self.apply_tool();
                }
            },
            Button::Right => {
                self.move_cursor(1, 0);
                if self.drawing && (self.tool == Tool::Pencil || self.tool == Tool::Eraser) {
                    self.apply_tool();
                }
            },
            Button::Confirm => {
                self.apply_tool();
            },
            Button::Triangle => {
                self.stop_drawing();
                self.tool = self.tool.next();
            },
            Button::Square => {
                self.cycle_color();
            },
            Button::Start => {
                self.stop_drawing();
                self.undo();
            },
            Button::Select => {
                self.stop_drawing();
                self.redo();
            },
            Button::Cancel => {
                self.stop_drawing();
                self.rebuild_display_lines();
                return AppAction::Exit;
            },
        }
        self.rebuild_display_lines();
        AppAction::None
    }

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, _fullscreen: bool) -> AppAction {
        // Map click coords to canvas coords based on scale.
        let scale_x = cw / self.canvas.width().max(1);
        let scale_y = ch / self.canvas.height().max(1);
        let scale = scale_x.min(scale_y).max(1);
        let canvas_x = lx / scale as i32;
        let canvas_y = ly / scale as i32;
        self.cursor_x = canvas_x.clamp(0, self.canvas.width() as i32 - 1);
        self.cursor_y = canvas_y.clamp(0, self.canvas.height() as i32 - 1);
        self.apply_tool();
        self.stop_drawing();
        self.rebuild_display_lines();
        AppAction::None
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> crate::error::Result<()> {
        // Draw the text info first using the standard helper.
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)?;

        // Calculate canvas rendering area below the text info.
        // Reserve space for 8 info lines + title bar.
        let info_lines = 9u32;
        let line_h = at.terminal_line_height.max(1);
        let canvas_top = cy + at.app.title_bar_height as i32 + (info_lines * line_h) as i32;
        let available_h = ch.saturating_sub(at.app.title_bar_height + info_lines * line_h + 16);
        let available_w = cw.saturating_sub(8);

        if available_h < 4 || available_w < 4 {
            return Ok(());
        }

        // Calculate scale factor to fit canvas.
        let scale_x = available_w / self.canvas.width().max(1);
        let scale_y = available_h / self.canvas.height().max(1);
        let scale = scale_x.min(scale_y).max(1);

        let canvas_px_w = self.canvas.width() * scale;
        let canvas_px_h = self.canvas.height() * scale;
        let offset_x = cx + (cw.saturating_sub(canvas_px_w) / 2) as i32;
        let offset_y = canvas_top;

        // Draw canvas background (checkerboard for transparency).
        backend.fill_rect(
            offset_x,
            offset_y,
            canvas_px_w,
            canvas_px_h,
            Color::rgb(200, 200, 200),
        )?;

        // Draw pixels scaled up.
        let flat = &self.canvas.pixels;
        for py in 0..self.canvas.height() {
            for px in 0..self.canvas.width() {
                let c = flat[(py * self.canvas.width() + px) as usize];
                if c.a == 0 {
                    continue;
                }
                backend.fill_rect(
                    offset_x + (px * scale) as i32,
                    offset_y + (py * scale) as i32,
                    scale,
                    scale,
                    Color::rgb(c.r, c.g, c.b),
                )?;
            }
        }

        // Draw grid overlay if enabled.
        if self.show_grid && scale >= 3 {
            let grid_color = Color::rgba(100, 100, 100, 80);
            for gx in 0..=self.canvas.width() {
                backend.fill_rect(
                    offset_x + (gx * scale) as i32,
                    offset_y,
                    1,
                    canvas_px_h,
                    grid_color,
                )?;
            }
            for gy in 0..=self.canvas.height() {
                backend.fill_rect(
                    offset_x,
                    offset_y + (gy * scale) as i32,
                    canvas_px_w,
                    1,
                    grid_color,
                )?;
            }
        }

        // Draw cursor.
        let cur_x = offset_x + (self.cursor_x as u32 * scale) as i32;
        let cur_y = offset_y + (self.cursor_y as u32 * scale) as i32;
        let cursor_color = Color::rgb(255, 0, 0);
        // Outline around cursor pixel.
        backend.fill_rect(cur_x, cur_y, scale, 1, cursor_color)?;
        backend.fill_rect(cur_x, cur_y + scale as i32 - 1, scale, 1, cursor_color)?;
        backend.fill_rect(cur_x, cur_y, 1, scale, cursor_color)?;
        backend.fill_rect(cur_x + scale as i32 - 1, cur_y, 1, scale, cursor_color)?;

        // Draw color swatch.
        let swatch_x = cx + 4;
        let swatch_y = canvas_top - (line_h as i32) - 2;
        backend.fill_rect(swatch_x, swatch_y, line_h, line_h, self.color)?;

        Ok(())
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    // -- Canvas creation --

    #[test]
    fn canvas_new_dimensions() {
        let c = Canvas::new(64, 48);
        assert_eq!(c.width(), 64);
        assert_eq!(c.height(), 48);
    }

    #[test]
    fn canvas_new_all_white() {
        let c = Canvas::new(4, 4);
        let white = Color::rgb(255, 255, 255);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(c.get_pixel(x, y), white);
            }
        }
    }

    #[test]
    fn canvas_new_one_layer() {
        let c = Canvas::new(8, 8);
        assert_eq!(c.layer_count(), 1);
        assert_eq!(c.active_layer(), 0);
    }

    // -- Pixel get/set --

    #[test]
    fn canvas_set_get_pixel() {
        let mut c = Canvas::new(8, 8);
        let red = Color::rgb(255, 0, 0);
        c.set_pixel(3, 4, red);
        assert_eq!(c.get_pixel(3, 4), red);
    }

    #[test]
    fn canvas_get_pixel_out_of_bounds() {
        let c = Canvas::new(4, 4);
        assert_eq!(c.get_pixel(10, 10), Color::rgba(0, 0, 0, 0));
    }

    #[test]
    fn canvas_set_pixel_out_of_bounds_noop() {
        let mut c = Canvas::new(4, 4);
        c.set_pixel(100, 100, Color::rgb(255, 0, 0));
        // Should not panic or corrupt data.
        assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 255, 255));
    }

    // -- Canvas fill/clear --

    #[test]
    fn canvas_fill() {
        let mut c = Canvas::new(4, 4);
        let blue = Color::rgb(0, 0, 255);
        c.fill(blue);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(c.get_pixel(x, y), blue);
            }
        }
    }

    #[test]
    fn canvas_clear_makes_transparent() {
        let mut c = Canvas::new(4, 4);
        c.clear();
        // Transparent over nothing = transparent.
        let px = c.get_pixel(0, 0);
        assert_eq!(px.a, 0);
    }

    // -- Layer operations --

    #[test]
    fn canvas_add_layer() {
        let mut c = Canvas::new(4, 4);
        let idx = c.add_layer("Layer 1");
        assert_eq!(idx, 1);
        assert_eq!(c.layer_count(), 2);
    }

    #[test]
    fn canvas_set_active_layer() {
        let mut c = Canvas::new(4, 4);
        c.add_layer("Top");
        c.set_active_layer(1);
        assert_eq!(c.active_layer(), 1);
    }

    #[test]
    fn canvas_set_active_layer_invalid() {
        let mut c = Canvas::new(4, 4);
        c.set_active_layer(99);
        assert_eq!(c.active_layer(), 0);
    }

    #[test]
    fn canvas_layer_name() {
        let c = Canvas::new(4, 4);
        assert_eq!(c.layer_name(0), "Background");
        assert_eq!(c.layer_name(99), "");
    }

    #[test]
    fn canvas_toggle_layer_visibility() {
        let mut c = Canvas::new(4, 4);
        // Fill background red.
        c.fill(Color::rgb(255, 0, 0));
        // Toggle off.
        c.toggle_layer_visibility(0);
        // Pixel should be transparent when only layer is hidden.
        assert_eq!(c.get_pixel(0, 0).a, 0);
        // Toggle back on.
        c.toggle_layer_visibility(0);
        assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 0, 0));
    }

    #[test]
    fn canvas_flatten_two_layers() {
        let mut c = Canvas::new(2, 2);
        // Background is white.
        c.add_layer("Top");
        c.set_active_layer(1);
        // Draw an opaque red pixel on top layer.
        c.set_pixel(0, 0, Color::rgb(255, 0, 0));
        // That pixel should be red (opaque overrides).
        assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 0, 0));
        // Others should be white (from background).
        assert_eq!(c.get_pixel(1, 0), Color::rgb(255, 255, 255));
    }

    #[test]
    fn canvas_flatten_hidden_layer_ignored() {
        let mut c = Canvas::new(2, 2);
        c.add_layer("Top");
        c.set_active_layer(1);
        c.set_pixel(0, 0, Color::rgb(255, 0, 0));
        c.toggle_layer_visibility(1);
        // Red pixel from hidden layer should not show.
        assert_eq!(c.get_pixel(0, 0), Color::rgb(255, 255, 255));
    }

    // -- Drawing primitives --

    #[test]
    fn draw_pixel_basic() {
        let mut buf = vec![Color::rgb(0, 0, 0); 16];
        let red = Color::rgb(255, 0, 0);
        draw_pixel(&mut buf, 4, 2, 3, red);
        assert_eq!(buf[3 * 4 + 2], red);
    }

    #[test]
    fn draw_pixel_negative_coords_noop() {
        let mut buf = vec![Color::rgb(0, 0, 0); 16];
        draw_pixel(&mut buf, 4, -1, -1, Color::rgb(255, 0, 0));
        // No change expected.
        assert_eq!(buf[0], Color::rgb(0, 0, 0));
    }

    #[test]
    fn draw_pixel_out_of_bounds_noop() {
        let mut buf = vec![Color::rgb(0, 0, 0); 16];
        draw_pixel(&mut buf, 4, 10, 10, Color::rgb(255, 0, 0));
        assert_eq!(buf[0], Color::rgb(0, 0, 0));
    }

    #[test]
    fn draw_line_horizontal() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let red = Color::rgb(255, 0, 0);
        draw_line(&mut buf, 8, 8, 1, 3, 5, 3, red, 1);
        for x in 1..=5 {
            assert_eq!(buf[3 * 8 + x], red, "pixel at ({x}, 3) should be red");
        }
    }

    #[test]
    fn draw_line_vertical() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let green = Color::rgb(0, 255, 0);
        draw_line(&mut buf, 8, 8, 2, 1, 2, 6, green, 1);
        for y in 1..=6 {
            assert_eq!(buf[y * 8 + 2], green, "pixel at (2, {y}) should be green");
        }
    }

    #[test]
    fn draw_line_diagonal() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let blue = Color::rgb(0, 0, 255);
        draw_line(&mut buf, 8, 8, 0, 0, 7, 7, blue, 1);
        // At minimum, endpoints should be set.
        assert_eq!(buf[0], blue);
        assert_eq!(buf[7 * 8 + 7], blue);
    }

    #[test]
    fn draw_line_single_point() {
        let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
        let c = Color::rgb(128, 128, 128);
        draw_line(&mut buf, 4, 4, 2, 2, 2, 2, c, 1);
        assert_eq!(buf[2 * 4 + 2], c);
    }

    #[test]
    fn draw_rect_outline() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let c = Color::rgb(255, 0, 0);
        draw_rect(&mut buf, 8, 8, 1, 1, 4, 3, c);
        // Top edge: y=1, x=1..4
        for x in 1..=4 {
            assert_eq!(buf[1 * 8 + x], c);
        }
        // Bottom edge: y=3, x=1..4
        for x in 1..=4 {
            assert_eq!(buf[3 * 8 + x], c);
        }
        // Left edge: x=1, y=1..3
        for y in 1..=3 {
            assert_eq!(buf[y * 8 + 1], c);
        }
        // Interior should be untouched.
        assert_eq!(buf[2 * 8 + 2], Color::rgb(0, 0, 0));
    }

    #[test]
    fn draw_rect_zero_size_noop() {
        let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
        draw_rect(&mut buf, 4, 4, 0, 0, 0, 0, Color::rgb(255, 0, 0));
        assert_eq!(buf[0], Color::rgb(0, 0, 0));
    }

    #[test]
    fn draw_filled_rect_basic() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let c = Color::rgb(0, 255, 0);
        draw_filled_rect(&mut buf, 8, 8, 2, 2, 3, 2, c);
        for y in 2..4 {
            for x in 2..5 {
                assert_eq!(buf[y * 8 + x], c, "pixel at ({x}, {y})");
            }
        }
    }

    #[test]
    fn draw_circle_zero_radius() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let c = Color::rgb(255, 0, 0);
        draw_circle(&mut buf, 8, 8, 4, 4, 0, c);
        assert_eq!(buf[4 * 8 + 4], c);
    }

    #[test]
    fn draw_circle_outline() {
        let mut buf = vec![Color::rgb(0, 0, 0); 16 * 16];
        let c = Color::rgb(0, 0, 255);
        draw_circle(&mut buf, 16, 16, 8, 8, 3, c);
        // Center should not be filled (outline only).
        assert_eq!(buf[8 * 16 + 8], Color::rgb(0, 0, 0));
        // Some point on the circle should be drawn.
        assert_eq!(buf[(8 - 3) * 16 + 8], c);
    }

    #[test]
    fn draw_filled_circle_basic() {
        let mut buf = vec![Color::rgb(0, 0, 0); 16 * 16];
        let c = Color::rgb(255, 128, 0);
        draw_filled_circle(&mut buf, 16, 16, 8, 8, 3, c);
        // Center should be filled.
        assert_eq!(buf[8 * 16 + 8], c);
        // Point exactly at radius should be filled.
        assert_eq!(buf[8 * 16 + 11], c); // (11, 8), dx=3
    }

    #[test]
    fn draw_filled_circle_zero_radius() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let c = Color::rgb(255, 0, 0);
        draw_filled_circle(&mut buf, 8, 8, 4, 4, 0, c);
        assert_eq!(buf[4 * 8 + 4], c);
    }

    // -- Flood fill --

    #[test]
    fn flood_fill_basic() {
        let mut buf = vec![Color::rgb(255, 255, 255); 4 * 4];
        let red = Color::rgb(255, 0, 0);
        flood_fill(&mut buf, 4, 4, 0, 0, red);
        for px in &buf {
            assert_eq!(*px, red);
        }
    }

    #[test]
    fn flood_fill_bounded() {
        // Create a 4x4 canvas with a barrier.
        let w = Color::rgb(255, 255, 255);
        let b = Color::rgb(0, 0, 0);
        #[rustfmt::skip]
        let mut buf = vec![
            w, w, b, w,
            w, w, b, w,
            b, b, b, w,
            w, w, w, w,
        ];
        let red = Color::rgb(255, 0, 0);
        flood_fill(&mut buf, 4, 4, 0, 0, red);
        // Top-left region should be red.
        assert_eq!(buf[0], red);
        assert_eq!(buf[1], red);
        assert_eq!(buf[4], red);
        assert_eq!(buf[5], red);
        // Beyond barrier should be unchanged.
        assert_eq!(buf[3], w);
        assert_eq!(buf[7], w);
    }

    #[test]
    fn flood_fill_same_color_noop() {
        let red = Color::rgb(255, 0, 0);
        let mut buf = vec![red; 4 * 4];
        flood_fill(&mut buf, 4, 4, 0, 0, red);
        // No infinite loop, same color fill is a no-op.
        assert_eq!(buf[0], red);
    }

    #[test]
    fn flood_fill_out_of_bounds() {
        let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
        // Should not panic.
        flood_fill(&mut buf, 4, 4, -1, -1, Color::rgb(255, 0, 0));
        flood_fill(&mut buf, 4, 4, 10, 10, Color::rgb(255, 0, 0));
    }

    // -- Brush --

    #[test]
    fn draw_brush_size_1() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let c = Color::rgb(255, 0, 0);
        draw_brush(&mut buf, 8, 8, 4, 4, 1, c);
        assert_eq!(buf[4 * 8 + 4], c);
    }

    #[test]
    fn draw_brush_size_3() {
        let mut buf = vec![Color::rgb(0, 0, 0); 8 * 8];
        let c = Color::rgb(0, 255, 0);
        draw_brush(&mut buf, 8, 8, 4, 4, 3, c);
        // Should fill a 3x3 area centered on (4,4).
        for dy in -1..=1i32 {
            for dx in -1..=1i32 {
                let x = (4 + dx) as usize;
                let y = (4 + dy) as usize;
                assert_eq!(buf[y * 8 + x], c, "pixel at ({x}, {y})");
            }
        }
    }

    #[test]
    fn draw_brush_at_edge() {
        let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
        let c = Color::rgb(255, 0, 0);
        // Brush at corner, some pixels out of bounds.
        draw_brush(&mut buf, 4, 4, 0, 0, 3, c);
        assert_eq!(buf[0], c);
        // Should not panic.
    }

    // -- Undo/redo --

    #[test]
    fn undo_restores_state() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        // Move cursor to (0, 0).
        for _ in 0..32 {
            app.handle_input(&Button::Left, &vfs);
        }
        for _ in 0..24 {
            app.handle_input(&Button::Up, &vfs);
        }
        let before = app.canvas.get_pixel(0, 0);
        // Draw at (0, 0).
        app.handle_input(&Button::Confirm, &vfs);
        let after = app.canvas.get_pixel(0, 0);
        assert_ne!(before, after);
        // Undo.
        app.handle_input(&Button::Start, &vfs);
        let restored = app.canvas.get_pixel(0, 0);
        assert_eq!(restored, before);
    }

    #[test]
    fn redo_after_undo() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        for _ in 0..32 {
            app.handle_input(&Button::Left, &vfs);
        }
        for _ in 0..24 {
            app.handle_input(&Button::Up, &vfs);
        }
        app.handle_input(&Button::Confirm, &vfs);
        let drawn = app.canvas.get_pixel(0, 0);
        app.handle_input(&Button::Start, &vfs);
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.canvas.get_pixel(0, 0), drawn);
    }

    #[test]
    fn undo_stack_limited() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        // Perform more than MAX_UNDO operations.
        for _ in 0..MAX_UNDO + 5 {
            app.handle_input(&Button::Confirm, &vfs);
            app.stop_drawing();
        }
        assert!(app.undo_stack.len() <= MAX_UNDO);
    }

    #[test]
    fn undo_empty_noop() {
        let mut app = PaintApp::new("/apps/paint");
        // Should not panic.
        app.undo();
        assert!(app.redo_stack.is_empty());
    }

    #[test]
    fn redo_empty_noop() {
        let mut app = PaintApp::new("/apps/paint");
        app.redo();
        assert!(app.undo_stack.is_empty());
    }

    // -- Color palette --

    #[test]
    fn palette_has_16_colors() {
        assert_eq!(palette().len(), 16);
    }

    #[test]
    fn palette_first_is_black() {
        assert_eq!(palette()[0], Color::rgb(0, 0, 0));
    }

    #[test]
    fn palette_second_is_white() {
        assert_eq!(palette()[1], Color::rgb(255, 255, 255));
    }

    #[test]
    fn palette_all_opaque() {
        for c in &palette() {
            assert_eq!(c.a, 255);
        }
    }

    // -- Tool cycling --

    #[test]
    fn tool_cycle_all() {
        let mut t = Tool::Pencil;
        let mut seen = Vec::new();
        for _ in 0..8 {
            seen.push(t);
            t = t.next();
        }
        assert_eq!(seen.len(), 8);
        // Should wrap back to pencil.
        assert_eq!(t, Tool::Pencil);
    }

    #[test]
    fn tool_names_nonempty() {
        for t in &Tool::ALL {
            assert!(!t.name().is_empty());
        }
    }

    // -- PaintApp state --

    #[test]
    fn paint_app_title_and_path() {
        let app = PaintApp::new("/apps/paint");
        assert_eq!(app.title(), "Paint");
        assert_eq!(app.path(), "/apps/paint");
    }

    #[test]
    fn paint_app_initial_cursor() {
        let app = PaintApp::new("/apps/paint");
        assert_eq!(app.cursor_x, (CANVAS_W / 2) as i32);
        assert_eq!(app.cursor_y, (CANVAS_H / 2) as i32);
    }

    #[test]
    fn paint_app_initial_tool() {
        let app = PaintApp::new("/apps/paint");
        assert_eq!(app.tool, Tool::Pencil);
    }

    #[test]
    fn paint_app_cursor_movement() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        let start_x = app.cursor_x;
        let start_y = app.cursor_y;
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.cursor_x, start_x + 1);
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.cursor_y, start_y + 1);
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.cursor_x, start_x);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.cursor_y, start_y);
    }

    #[test]
    fn paint_app_cursor_clamp_bounds() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        // Move far left/up to hit (0, 0).
        for _ in 0..100 {
            app.handle_input(&Button::Left, &vfs);
            app.handle_input(&Button::Up, &vfs);
        }
        assert_eq!(app.cursor_x, 0);
        assert_eq!(app.cursor_y, 0);
        // Move far right/down.
        for _ in 0..200 {
            app.handle_input(&Button::Right, &vfs);
            app.handle_input(&Button::Down, &vfs);
        }
        assert_eq!(app.cursor_x, CANVAS_W as i32 - 1);
        assert_eq!(app.cursor_y, CANVAS_H as i32 - 1);
    }

    #[test]
    fn paint_app_tool_cycle_via_input() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        assert_eq!(app.tool, Tool::Pencil);
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.tool, Tool::Line);
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.tool, Tool::Rectangle);
    }

    #[test]
    fn paint_app_color_cycle() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        let pal = palette();
        assert_eq!(app.color, pal[0]);
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.color, pal[1]);
    }

    #[test]
    fn paint_app_brush_size_cycle() {
        let mut app = PaintApp::new("/apps/paint");
        assert_eq!(app.brush_size, 1);
        app.cycle_brush_size();
        assert_eq!(app.brush_size, 2);
        app.cycle_brush_size();
        assert_eq!(app.brush_size, 3);
        // Cycle through to 5, then back to 1.
        app.cycle_brush_size();
        app.cycle_brush_size();
        assert_eq!(app.brush_size, 5);
        app.cycle_brush_size();
        assert_eq!(app.brush_size, 1);
    }

    #[test]
    fn paint_app_grid_toggle() {
        let mut app = PaintApp::new("/apps/paint");
        assert!(!app.show_grid);
        app.show_grid = true;
        assert!(app.show_grid);
        app.show_grid = false;
        assert!(!app.show_grid);
    }

    #[test]
    fn paint_app_start_undoes() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        // Start button should trigger undo (no-op when stack
        // is empty, but should not panic).
        let action = app.handle_input(&Button::Start, &vfs);
        assert_eq!(action, AppAction::None);
    }

    #[test]
    fn paint_app_select_redoes() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        // Select button should trigger redo (no-op when stack
        // is empty, but should not panic).
        let action = app.handle_input(&Button::Select, &vfs);
        assert_eq!(action, AppAction::None);
    }

    #[test]
    fn paint_app_cancel_exits() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn paint_app_display_lines_nonempty() {
        let app = PaintApp::new("/apps/paint");
        assert!(!app.lines().is_empty());
        assert!(app.lines().iter().any(|l| l.contains("Paint")));
    }

    #[test]
    fn paint_app_display_lines_show_tool() {
        let app = PaintApp::new("/apps/paint");
        assert!(app.lines().iter().any(|l| l.contains("Pencil")));
    }

    #[test]
    fn paint_app_downcast() {
        let app = PaintApp::new("/apps/paint");
        let any = app.as_any();
        assert!(any.downcast_ref::<PaintApp>().is_some());
    }

    #[test]
    fn paint_app_downcast_mut() {
        let mut app = PaintApp::new("/apps/paint");
        let any = app.as_any_mut();
        assert!(any.downcast_mut::<PaintApp>().is_some());
    }

    // -- Edge cases --

    #[test]
    fn draw_at_canvas_boundary() {
        let mut buf = vec![Color::rgb(0, 0, 0); 4 * 4];
        let c = Color::rgb(255, 0, 0);
        // All four corners.
        draw_pixel(&mut buf, 4, 0, 0, c);
        draw_pixel(&mut buf, 4, 3, 0, c);
        draw_pixel(&mut buf, 4, 0, 3, c);
        draw_pixel(&mut buf, 4, 3, 3, c);
        assert_eq!(buf[0], c);
        assert_eq!(buf[3], c);
        assert_eq!(buf[12], c);
        assert_eq!(buf[15], c);
    }

    #[test]
    fn line_with_brush_size() {
        let mut buf = vec![Color::rgb(0, 0, 0); 16 * 16];
        let c = Color::rgb(255, 0, 0);
        draw_line(&mut buf, 16, 16, 2, 8, 14, 8, c, 3);
        // Center row should be filled.
        for x in 2..=14 {
            assert_eq!(buf[8 * 16 + x], c, "center at ({x}, 8)");
        }
        // Row above should also be partially filled
        // (brush extends 1 above).
        assert_eq!(buf[7 * 16 + 2], c);
    }

    #[test]
    fn alpha_blend_opaque_over_transparent() {
        let src = Color::rgb(255, 0, 0);
        let dst = Color::rgba(0, 0, 0, 0);
        let result = alpha_blend(src, dst, 255);
        assert_eq!(result.r, 255);
        assert_eq!(result.g, 0);
        assert_eq!(result.b, 0);
        assert_eq!(result.a, 255);
    }

    #[test]
    fn alpha_blend_transparent_over_opaque() {
        let src = Color::rgba(0, 0, 0, 0);
        let dst = Color::rgb(0, 255, 0);
        let result = alpha_blend(src, dst, 255);
        assert_eq!(result, dst);
    }

    #[test]
    fn alpha_blend_semi_transparent() {
        let src = Color::rgba(255, 0, 0, 128);
        let dst = Color::rgb(0, 0, 255);
        let result = alpha_blend(src, dst, 255);
        // Should be a blend of red and blue.
        assert!(result.r > 0);
        assert!(result.b > 0);
        assert!(result.a > 128);
    }

    #[test]
    fn canvas_1x1() {
        let mut c = Canvas::new(1, 1);
        let red = Color::rgb(255, 0, 0);
        c.set_pixel(0, 0, red);
        assert_eq!(c.get_pixel(0, 0), red);
    }

    #[test]
    fn encode_bmp_valid_header() {
        let pixels = vec![Color::rgb(255, 0, 0); 4];
        let bmp = encode_bmp(&pixels, 2, 2);
        assert_eq!(&bmp[0..2], b"BM");
        // File size: 54 header + 2*2*4 = 70.
        let file_size = u32::from_le_bytes([bmp[2], bmp[3], bmp[4], bmp[5]]);
        assert_eq!(file_size, 70);
    }

    #[test]
    fn save_to_vfs_creates_file() {
        let app = PaintApp::new("/apps/paint");
        let mut vfs = make_vfs();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home").unwrap();
        crate::vfs::Vfs::mkdir(&mut vfs, "/home/user").unwrap();
        assert!(app.save_to_vfs(&mut vfs));
        assert!(vfs.exists("/home/user/pictures/paint_64x48.bmp"));
    }

    #[test]
    fn new_canvas_resets_state() {
        let mut app = PaintApp::new("/apps/paint");
        let vfs = make_vfs();
        // Draw something and add undo entries.
        app.handle_input(&Button::Confirm, &vfs);
        app.stop_drawing();
        assert!(!app.undo_stack.is_empty());
        // Reset canvas.
        app.new_canvas(32, 32);
        assert!(app.undo_stack.is_empty());
        assert_eq!(app.canvas.width(), 32);
        assert_eq!(app.canvas.height(), 32);
    }

    #[test]
    fn new_canvas_clamps_size() {
        let mut app = PaintApp::new("/apps/paint");
        app.new_canvas(4, 4); // below minimum
        assert_eq!(app.canvas.width(), 8);
        assert_eq!(app.canvas.height(), 8);
        app.new_canvas(999, 999); // above maximum
        assert_eq!(app.canvas.width(), 256);
        assert_eq!(app.canvas.height(), 256);
    }
}
