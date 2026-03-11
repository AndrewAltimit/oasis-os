use oasis_types::backend::Color;

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
    pub(crate) const ALL: [Tool; 8] = [
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
