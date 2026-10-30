//! Paint/Drawing application for OASIS OS.
//!
//! A pixel-art drawing app with multi-layer canvas, multiple drawing
//! tools (pencil, line, rectangle, circle, eraser, flood fill),
//! a 16-color palette, undo/redo, grid overlay, and BMP save / open.
//! The canvas is rendered scaled up to fit the content area; the menu
//! bar, info strip, canvas and palette positions all come from
//! `layout::PaintLayout`, which both drawing and hit-testing use.
//!
//! Pointer input: hosts only forward discrete clicks to apps (no drag /
//! pointer-move events), so freehand strokes are drawn click-by-click or
//! with the d-pad while a stroke is active; shape tools take two clicks
//! (start point, end point).

pub mod canvas;
mod file_ops;
pub mod io;
pub(crate) mod layout;
pub mod palette;
mod render;
pub mod tools;

use oasis_app_core::{App, AppAction, ContentState};
use oasis_skin::ActiveTheme;
use oasis_types::backend::{Color, SdiBackend};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_ui::menu_bar::{Menu, MenuBar, MenuEntry, MenuHit};
use oasis_vfs::Vfs;

pub use canvas::{Canvas, Layer, UndoEntry};
pub use io::{PICTURES_DIR, decode_bmp};
pub use palette::palette;
pub use tools::{
    Tool, draw_brush, draw_circle, draw_filled_circle, draw_filled_rect, draw_line, draw_pixel,
    draw_rect, flood_fill,
};

use canvas::{CANVAS_H, CANVAS_W, MAX_UNDO, encode_bmp};
use layout::PaintLayout;
use palette::PALETTE_NAMES;

// ---------------------------------------------------------------
// PaintApp
// ---------------------------------------------------------------

/// File work that needs the VFS, queued by handlers that don't get one
/// (clicks) or need mutable access (saving). Drained by
/// `App::apply_vfs_ops`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PaintIo {
    /// Save to the current file, or a new unique file if untitled.
    Save,
    /// Save to a new unique file.
    SaveAs,
}

/// File > Open picker state.
#[derive(Debug, Clone, Default)]
struct Picker {
    /// BMP paths found in [`PICTURES_DIR`]; `None` until listed.
    files: Option<Vec<String>>,
    /// Selected index into `files`.
    selected: usize,
    /// First visible row.
    scroll: usize,
    /// Path chosen by a click, loaded as soon as a VFS is available.
    load: Option<String>,
}

/// The Paint/Drawing application.
#[derive(Debug)]
pub struct PaintApp {
    content: ContentState,
    /// Content-area size recorded by the last `App::layout` (click
    /// hit-testing).
    content_size: (u32, u32),
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
    /// Top menu bar (File / Edit / Tool / Brush / Layer / View).
    menu: MenuBar,
    /// File the canvas was loaded from / last saved to.
    file_path: Option<String>,
    /// Unsaved changes since the last new / open / save.
    modified: bool,
    /// One-line feedback (save result, errors).
    status: Option<String>,
    /// Queued save work (needs `&mut dyn Vfs`).
    pending_io: Vec<PaintIo>,
    /// Open picker, when File > Open is active.
    picker: Option<Picker>,
    /// Visible picker rows of the last `App::layout` (for keyboard and
    /// wheel scrolling).
    picker_rows: usize,
}

/// Build the Paint menu bar.
fn default_menu_bar() -> MenuBar {
    let tools = Tool::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| MenuEntry::action(t.name(), format!("tool.{i}")))
        .collect();
    let mut brush: Vec<MenuEntry> = (1..=5)
        .map(|n| MenuEntry::action(format!("Size {n}"), format!("brush.size.{n}")))
        .collect();
    brush.push(MenuEntry::Separator);
    brush.push(MenuEntry::action("Next Colour", "brush.color.next").with_shortcut("\u{25a1}"));
    brush.push(MenuEntry::action("Prev Colour", "brush.color.prev"));
    MenuBar::new(vec![
        Menu::new(
            "File",
            vec![
                MenuEntry::action("New", "file.new").with_shortcut("Ctrl+N"),
                MenuEntry::action("Open...", "file.open").with_shortcut("Ctrl+O"),
                MenuEntry::action("Save", "file.save").with_shortcut("Ctrl+S"),
                MenuEntry::action("Save As", "file.save_as").with_shortcut("Ctrl+Sh+S"),
                MenuEntry::Separator,
                MenuEntry::action("Exit", "file.exit").with_shortcut("Esc"),
            ],
        ),
        Menu::new(
            "Edit",
            vec![
                MenuEntry::action("Undo", "edit.undo").with_shortcut("Ctrl+Z"),
                MenuEntry::action("Redo", "edit.redo").with_shortcut("Ctrl+Y"),
                MenuEntry::Separator,
                MenuEntry::action("Clear Layer", "edit.clear"),
            ],
        ),
        Menu::new("Tool", tools),
        Menu::new("Brush", brush),
        Menu::new(
            "Layer",
            vec![
                MenuEntry::action("Add Layer", "layer.add"),
                MenuEntry::action("Next Layer", "layer.next"),
                MenuEntry::action("Show/Hide", "layer.toggle"),
            ],
        ),
        Menu::new(
            "View",
            vec![MenuEntry::action("Grid", "view.grid").with_shortcut("G")],
        ),
    ])
}

impl PaintApp {
    /// Create a new Paint application.
    pub fn new(path: &str) -> Self {
        let pal = palette();
        let mut app = Self {
            content: ContentState::new("Paint", path),
            content_size: (0, 0),
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
            menu: default_menu_bar(),
            file_path: None,
            modified: false,
            status: None,
            pending_io: Vec::new(),
            picker: None,
            picker_rows: 8,
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
            self.modified = true;
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
            self.modified = true;
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
            self.modified = true;
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
                    // The undo snapshot is taken when the shape is
                    // committed (`finish_shape`), so a cancelled shape
                    // leaves history and the modified flag alone.
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
        self.push_undo();
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

    /// Display name of the current file (`untitled` before first save).
    fn file_label(&self) -> String {
        let name = self
            .file_path
            .as_deref()
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or("untitled");
        let star = if self.modified { "*" } else { "" };
        format!("{name}{star}")
    }

    /// Name of the current palette colour.
    fn color_name(&self) -> &'static str {
        PALETTE_NAMES
            .get(self.palette_index)
            .copied()
            .unwrap_or("Custom")
    }

    /// Rebuild the text display lines.
    fn rebuild_display_lines(&mut self) {
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
            // Canvas dimensions only — the app title already shows in the
            // WM / app-chrome title bar.
            format!(
                "Canvas: {}x{}  File: {}",
                self.canvas.width(),
                self.canvas.height(),
                self.file_label()
            ),
            "\u{2500}".repeat(30),
            format!(
                "  Tool: {}  Color: {} ({})  Size: {}",
                self.tool.name(),
                "\u{2588}\u{2588}",
                self.color_name(),
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
        if let Some(status) = &self.status {
            self.display_lines.push(format!("  {status}"));
        }
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
        self.select_color((self.palette_index + 1) % palette().len());
    }

    /// Select palette entry `index`.
    fn select_color(&mut self, index: usize) {
        let pal = palette();
        if let Some(c) = pal.get(index) {
            self.palette_index = index;
            self.color = *c;
        }
    }

    /// Cycle brush size 1-5.
    pub fn cycle_brush_size(&mut self) {
        self.brush_size = (self.brush_size % 5) + 1;
    }

    /// Create a new canvas with the given dimensions.
    pub fn new_canvas(&mut self, width: u32, height: u32) {
        let w = width.clamp(8, io::MAX_SIDE);
        let h = height.clamp(8, io::MAX_SIDE);
        self.canvas = Canvas::new(w, h);
        self.reset_after_canvas_change();
        self.file_path = None;
        self.rebuild_display_lines();
    }

    /// Reset cursor / history after the canvas was replaced.
    fn reset_after_canvas_change(&mut self) {
        self.cursor_x = (self.canvas.width() / 2) as i32;
        self.cursor_y = (self.canvas.height() / 2) as i32;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.drawing = false;
        self.drag_start = None;
        self.modified = false;
    }

    /// Dispatch a menu-bar / shortcut action by id.
    fn run_menu_action(&mut self, id: &str) -> AppAction {
        self.stop_drawing();
        match id {
            "file.new" => {
                self.new_canvas(self.canvas.width(), self.canvas.height());
                self.status = Some("New canvas".to_string());
            },
            "file.open" => self.begin_open(),
            "file.save" => self.pending_io.push(PaintIo::Save),
            "file.save_as" => self.pending_io.push(PaintIo::SaveAs),
            "file.exit" => return AppAction::Exit,
            "edit.undo" => self.undo(),
            "edit.redo" => self.redo(),
            "edit.clear" => {
                self.push_undo();
                if self.canvas.active_layer() == 0 {
                    self.canvas.fill(Color::rgb(255, 255, 255));
                } else {
                    self.canvas.clear();
                }
            },
            "layer.add" => {
                let n = self.canvas.layer_count();
                let idx = self.canvas.add_layer(&format!("Layer {n}"));
                self.canvas.set_active_layer(idx);
                self.modified = true;
            },
            "layer.next" => {
                let next = (self.canvas.active_layer() + 1) % self.canvas.layer_count().max(1);
                self.canvas.set_active_layer(next);
            },
            "layer.toggle" => {
                self.canvas
                    .toggle_layer_visibility(self.canvas.active_layer());
                self.modified = true;
            },
            "view.grid" => self.show_grid = !self.show_grid,
            "brush.color.next" => self.cycle_color(),
            "brush.color.prev" => {
                let len = palette().len();
                self.select_color((self.palette_index + len - 1) % len);
            },
            _ => {
                if let Some(n) = id.strip_prefix("brush.size.").and_then(|n| n.parse().ok()) {
                    self.brush_size = n;
                } else if let Some(t) = id
                    .strip_prefix("tool.")
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| Tool::ALL.get(i))
                {
                    self.tool = *t;
                }
            },
        }
        self.rebuild_display_lines();
        AppAction::None
    }

    /// Map a keyboard shortcut to a menu action id.
    fn shortcut_action(key: &Key, mods: Modifiers) -> Option<&'static str> {
        let cmd = mods.ctrl() || mods.super_key();
        let lower = match key {
            Key::Char(c) => c.to_ascii_lowercase(),
            _ => return None,
        };
        if cmd && !mods.alt() {
            return match (lower, mods.shift()) {
                ('s', true) => Some("file.save_as"),
                ('s', false) => Some("file.save"),
                ('n', _) => Some("file.new"),
                ('o', _) => Some("file.open"),
                ('z', true) | ('y', _) => Some("edit.redo"),
                ('z', false) => Some("edit.undo"),
                _ => None,
            };
        }
        if mods.has_command() {
            return None;
        }
        match lower {
            'g' => Some("view.grid"),
            _ => None,
        }
    }
}

impl App for PaintApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        if self.picker.is_some() {
            self.handle_picker_input(button, vfs, self.picker_rows);
            self.rebuild_display_lines();
            return AppAction::None;
        }
        self.status = None;
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
                if self.menu.is_open() {
                    self.menu.close();
                    return AppAction::None;
                }
                self.stop_drawing();
                self.rebuild_display_lines();
                return AppAction::Exit;
            },
        }
        self.rebuild_display_lines();
        AppAction::None
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, vfs: &dyn Vfs) -> Option<AppAction> {
        if self.picker.is_some() {
            // Arrows / Enter / Esc reach the picker via their d-pad twins.
            return None;
        }
        match key {
            Key::Char('[') if !mods.has_command() => {
                self.brush_size = self.brush_size.saturating_sub(1).max(1);
            },
            Key::Char(']') if !mods.has_command() => {
                self.brush_size = (self.brush_size + 1).min(5);
            },
            _ => {
                let id = Self::shortcut_action(key, mods)?;
                let action = self.run_menu_action(id);
                // Ctrl+O opens the picker: list the files right away.
                self.service_picker(vfs);
                return Some(action);
            },
        }
        self.rebuild_display_lines();
        Some(AppAction::None)
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        let mut changed = self.service_picker(vfs);
        for op in std::mem::take(&mut self.pending_io) {
            let result = match op {
                PaintIo::Save => self.save_to_vfs(vfs),
                PaintIo::SaveAs => self.save_as_to_vfs(vfs),
            };
            self.report(result.map(|p| format!("Saved {p}")));
            changed = true;
        }
        if changed {
            self.rebuild_display_lines();
        }
        changed
    }

    fn layout(&mut self, cw: u32, ch: u32, _at: &ActiveTheme) {
        self.content_size = (cw, ch);
        let l = PaintLayout::compute(0, 0, cw, ch, self.canvas.width(), self.canvas.height());
        self.picker_rows = l.picker_rows();
    }

    /// The wheel scrolls the Open picker's list; over the canvas it does
    /// nothing (Up / Down would move the drawing cursor).
    fn handle_scroll(&mut self, delta: i32, vfs: &dyn Vfs) -> bool {
        if self.picker.is_none() {
            return false;
        }
        let button = if delta < 0 { Button::Up } else { Button::Down };
        for _ in 0..delta.unsigned_abs().min(oasis_app_core::MAX_WHEEL_STEPS) {
            self.handle_picker_input(&button, vfs, self.picker_rows);
        }
        delta != 0
    }

    fn handle_click(&mut self, lx: i32, ly: i32, vfs: &dyn Vfs) -> AppAction {
        let (cw, ch) = self.content_size;
        let action = self.click(lx, ly, cw, ch);
        // A click can open the picker or choose a file: list / load now.
        self.service_picker(vfs);
        action
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        self.draw_paint(cx, cy, cw, ch, backend, at)
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }
}

impl PaintApp {
    /// Content-area click (menu, tools, palette, picker, canvas).
    fn click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32) -> AppAction {
        let l = PaintLayout::compute(0, 0, cw, ch, self.canvas.width(), self.canvas.height());

        // 1. Menu bar and its drop-down.
        match self
            .menu
            .hit_test(lx, ly, l.menu.x, l.menu.y, l.menu.w, l.menu.h)
        {
            MenuHit::Label(i) => {
                if self.menu.open == Some(i) {
                    self.menu.close();
                } else {
                    self.menu.open = Some(i);
                    self.menu.hovered_item = None;
                }
                return AppAction::None;
            },
            MenuHit::Item { id } => {
                self.menu.close();
                return self.run_menu_action(&id);
            },
            MenuHit::NoOp => return AppAction::None,
            MenuHit::Outside => {
                if self.menu.is_open() {
                    self.menu.close();
                    return AppAction::None;
                }
            },
        }

        // 2. Open picker is modal: a row click loads, anything else closes.
        if let Some(picker) = &mut self.picker {
            let count = picker.files.as_ref().map_or(0, Vec::len);
            match l.picker_row_at(lx, ly).map(|r| r + picker.scroll) {
                Some(idx) if idx < count => {
                    picker.selected = idx;
                    picker.load = picker.files.as_ref().and_then(|f| f.get(idx)).cloned();
                },
                Some(_) => {},
                None => self.picker = None,
            }
            return AppAction::None;
        }

        // 3. Palette swatches.
        if let Some(i) = l.swatch_at(lx, ly) {
            self.select_color(i);
            self.rebuild_display_lines();
            return AppAction::None;
        }

        // 4. Canvas.
        let Some((px, py)) = l.canvas_pixel_at(lx, ly) else {
            return AppAction::None;
        };
        self.status = None;
        self.cursor_x = px;
        self.cursor_y = py;
        self.apply_tool();
        // Freehand tools finish the stroke per click (each click is its
        // own undo step); shape tools keep their start point for the
        // second click.
        if matches!(self.tool, Tool::Pencil | Tool::Eraser | Tool::Fill) {
            self.stop_drawing();
        }
        self.rebuild_display_lines();
        AppAction::None
    }
}

#[cfg(test)]
mod tests;
