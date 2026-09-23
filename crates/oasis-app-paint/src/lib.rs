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
pub mod io;
pub(crate) mod layout;
pub mod palette;
mod render;
pub mod tools;

use std::any::Any;
use std::cell::Cell;

use oasis_app_core::render::{hide_app_sdi, render_app_chrome, render_content_sdi};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
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
    /// Visible picker rows from the last draw (for keyboard scrolling;
    /// `Cell` so the `&self` renderer can update it).
    cached_picker_rows: Cell<usize>,
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
            cached_picker_rows: Cell::new(8),
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

    /// Encode the flattened canvas and write it to `path`, creating the
    /// parent folder if needed.
    fn write_bmp(&mut self, vfs: &mut dyn Vfs, path: &str) -> Result<(), String> {
        if let Some((dir, _)) = path.rsplit_once('/') {
            io::ensure_dir(vfs, dir)?;
        }
        let w = self.canvas.width();
        let h = self.canvas.height();
        let bmp = encode_bmp(&self.canvas.flatten(), w, h);
        vfs.write(path, &bmp).map_err(|e| e.to_string())?;
        self.file_path = Some(path.to_string());
        self.modified = false;
        Ok(())
    }

    /// Save the canvas as a BMP: to the file it was opened from / last
    /// saved to, or to a new unique `paint_NNN.bmp` in [`PICTURES_DIR`].
    /// Returns the path written.
    pub fn save_to_vfs(&mut self, vfs: &mut dyn Vfs) -> Result<String, String> {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => io::unique_save_path(vfs),
        };
        self.write_bmp(vfs, &path)?;
        Ok(path)
    }

    /// Save the canvas to a new unique file (never overwrites). Returns
    /// the path written.
    pub fn save_as_to_vfs(&mut self, vfs: &mut dyn Vfs) -> Result<String, String> {
        let path = io::unique_save_path(vfs);
        self.write_bmp(vfs, &path)?;
        Ok(path)
    }

    /// Replace the canvas with the BMP at `path`.
    pub fn open_from_vfs(&mut self, vfs: &dyn Vfs, path: &str) -> Result<(), String> {
        let data = vfs.read(path).map_err(|e| e.to_string())?;
        let (w, h, pixels) = decode_bmp(&data)?;
        self.canvas = Canvas::from_pixels(w, h, pixels);
        self.reset_after_canvas_change();
        self.file_path = Some(path.to_string());
        Ok(())
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

    /// Record the outcome of a file operation in the status line.
    fn report(&mut self, result: Result<String, String>) {
        self.status = Some(match result {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        });
    }

    /// Open the File > Open picker (listing happens once a VFS is at hand).
    fn begin_open(&mut self) {
        self.stop_drawing();
        self.picker = Some(Picker::default());
    }

    /// Fill in the picker listing and perform a click-chosen load. Needs
    /// only read access, so it runs from `refresh`, input handlers and
    /// `apply_vfs_ops` alike.
    fn service_picker(&mut self, vfs: &dyn Vfs) -> bool {
        let Some(picker) = &mut self.picker else {
            return false;
        };
        let mut changed = false;
        if picker.files.is_none() {
            picker.files = Some(io::list_bmps(vfs));
            changed = true;
        }
        if let Some(path) = picker.load.take() {
            self.picker = None;
            let result = self.open_from_vfs(vfs, &path).map(|()| {
                let name = path.rsplit('/').next().unwrap_or(&path);
                format!("Opened {name}")
            });
            self.report(result);
            changed = true;
        }
        if changed {
            self.rebuild_display_lines();
        }
        changed
    }

    /// Gamepad / keyboard navigation inside the Open picker.
    fn handle_picker_input(&mut self, button: &Button, vfs: &dyn Vfs, rows: usize) {
        self.service_picker(vfs);
        let Some(picker) = &mut self.picker else {
            return;
        };
        let count = picker.files.as_ref().map_or(0, Vec::len);
        match button {
            Button::Up => picker.selected = picker.selected.saturating_sub(1),
            Button::Down if picker.selected + 1 < count => picker.selected += 1,
            Button::Confirm => {
                if let Some(path) = picker.files.as_ref().and_then(|f| f.get(picker.selected)) {
                    picker.load = Some(path.clone());
                    self.service_picker(vfs);
                }
                return;
            },
            Button::Cancel => {
                self.picker = None;
                return;
            },
            _ => {},
        }
        let rows = rows.max(1);
        if picker.selected < picker.scroll {
            picker.scroll = picker.selected;
        } else if picker.selected >= picker.scroll + rows {
            picker.scroll = picker.selected + 1 - rows;
        }
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
            let rows = self.cached_picker_rows.get();
            self.handle_picker_input(button, vfs, rows);
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

    fn handle_key(&mut self, key: &Key, mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
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
                return Some(self.run_menu_action(id));
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

    fn refresh(&mut self, vfs: &dyn Vfs) {
        self.service_picker(vfs);
    }

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, _fullscreen: bool) -> AppAction {
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
    ) -> oasis_types::error::Result<()> {
        self.draw_paint(cx, cy, cw, ch, backend, at)
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
    use canvas::alpha_blend;
    use oasis_vfs::MemoryVfs;

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
            assert_eq!(buf[8 + x], c);
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
        // The heading shows canvas dimensions, not the app title — the
        // title already lives in the WM / app-chrome title bar.
        assert!(app.lines().iter().any(|l| l.contains("Canvas:")));
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
        let mut app = PaintApp::new("/apps/paint");
        let mut vfs = make_vfs();
        // The pictures folder (and its parents) are created on demand.
        let path = app.save_to_vfs(&mut vfs).expect("save");
        assert_eq!(path, "/home/user/pictures/paint_001.bmp");
        assert!(vfs.exists(&path));
    }

    #[test]
    fn save_reuses_file_but_save_as_never_overwrites() {
        let mut app = PaintApp::new("/apps/paint");
        let mut vfs = make_vfs();
        let first = app.save_to_vfs(&mut vfs).expect("save");
        // Save again: same file.
        assert_eq!(app.save_to_vfs(&mut vfs).expect("save"), first);
        // Save As: fresh unique file.
        let second = app.save_as_to_vfs(&mut vfs).expect("save as");
        assert_ne!(second, first);
        assert_eq!(second, "/home/user/pictures/paint_002.bmp");
        // New canvas is untitled again and gets yet another name.
        app.new_canvas(16, 16);
        let third = app.save_to_vfs(&mut vfs).expect("save");
        assert_eq!(third, "/home/user/pictures/paint_003.bmp");
    }

    /// Content rect used by the click tests (a typical window).
    const CW: u32 = 460;
    const CH: u32 = 240;

    fn click_canvas_pixel(app: &mut PaintApp, px: i32, py: i32) {
        let l = PaintLayout::compute(0, 0, CW, CH, app.canvas.width(), app.canvas.height());
        let r = l.pixel_rect(px, py);
        // Aim at the middle of the on-screen cell.
        app.handle_click(r.x + r.w as i32 / 2, r.y + r.h as i32 / 2, CW, CH, false);
    }

    #[test]
    fn click_on_screen_pixel_sets_matching_cursor() {
        let mut app = PaintApp::new("/apps/paint");
        let red = palette()[2];
        app.select_color(2);
        for (px, py) in [(0, 0), (10, 7), (63, 47), (31, 20)] {
            click_canvas_pixel(&mut app, px, py);
            assert_eq!((app.cursor_x, app.cursor_y), (px, py));
            assert_eq!(app.canvas.get_pixel(px as u32, py as u32), red);
        }
    }

    #[test]
    fn click_matches_windowed_draw_offset() {
        // Draw into a recording backend and check the cursor outline sits
        // exactly around the clicked canvas cell.
        let mut app = PaintApp::new("/apps/paint");
        click_canvas_pixel(&mut app, 5, 6);
        let l = PaintLayout::compute(0, 0, CW, CH, app.canvas.width(), app.canvas.height());
        let cell = l.pixel_rect(5, 6);
        let mut backend = oasis_test_backend::RecordingBackend::new(CW, CH);
        let at = ActiveTheme::default();
        app.draw_windowed(0, 0, CW, CH, &mut backend, &at)
            .expect("draw");
        let accent = render::PaintColors::from_theme(&at).cursor;
        // Top edge of the cursor outline hugs the clicked cell.
        let expected = oasis_test_backend::DrawCommand::FillRect {
            x: cell.x - 1,
            y: cell.y - 1,
            w: cell.w + 2,
            h: 1,
            color: accent,
        };
        assert!(backend.commands().contains(&expected));
    }

    #[test]
    fn clicks_outside_canvas_do_not_paint() {
        let mut app = PaintApp::new("/apps/paint");
        let before = app.canvas.pixels.clone();
        app.handle_click(1, CH as i32 - 30, CW, CH, false);
        assert_eq!(app.canvas.pixels, before);
        assert!(app.undo_stack.is_empty());
    }

    #[test]
    fn palette_click_selects_color() {
        let mut app = PaintApp::new("/apps/paint");
        let l = PaintLayout::compute(0, 0, CW, CH, 64, 48);
        let r = l.swatch_rect(4);
        app.handle_click(r.x + 1, r.y + 1, CW, CH, false);
        assert_eq!(app.palette_index, 4);
        assert_eq!(app.color, palette()[4]);
    }

    #[test]
    fn shape_tool_takes_two_clicks() {
        let mut app = PaintApp::new("/apps/paint");
        app.tool = Tool::Line;
        click_canvas_pixel(&mut app, 2, 2);
        assert_eq!(app.drag_start, Some((2, 2)));
        click_canvas_pixel(&mut app, 8, 2);
        assert_eq!(app.drag_start, None);
        for x in 2..=8 {
            assert_eq!(app.canvas.get_pixel(x, 2), app.color);
        }
    }

    #[test]
    fn save_then_open_round_trip() {
        let mut vfs = make_vfs();
        let mut app = PaintApp::new("/apps/paint");
        app.select_color(4);
        click_canvas_pixel(&mut app, 3, 4);
        let drawn = app.canvas.pixels.clone();

        // Ctrl+S queues the save; the host applies it with a mutable VFS.
        let consumed = app.handle_key(&Key::Char('s'), Modifiers::CTRL, &vfs);
        assert_eq!(consumed, Some(AppAction::None));
        assert!(app.apply_vfs_ops(&mut vfs));
        let path = app.file_path.clone().expect("saved path");
        assert!(!app.modified);

        // A fresh app opens it through File > Open.
        let mut other = PaintApp::new("/apps/paint");
        other.run_menu_action("file.open");
        other.refresh(&vfs);
        let files = other
            .picker
            .as_ref()
            .and_then(|p| p.files.clone())
            .expect("listing");
        assert_eq!(files, vec![path.clone()]);
        other.handle_input(&Button::Confirm, &vfs);
        assert!(other.picker.is_none());
        assert_eq!(other.file_path.as_deref(), Some(path.as_str()));
        assert_eq!(other.canvas.pixels, drawn);
        assert_eq!(other.canvas.get_pixel(3, 4), palette()[4]);
    }

    #[test]
    fn picker_row_click_loads_file() {
        let mut vfs = make_vfs();
        let mut app = PaintApp::new("/apps/paint");
        app.new_canvas(16, 8);
        app.save_to_vfs(&mut vfs).expect("save");
        let mut other = PaintApp::new("/apps/paint");
        other.begin_open();
        other.apply_vfs_ops(&mut vfs);
        let l = PaintLayout::compute(0, 0, CW, CH, 64, 48);
        let p = l.picker();
        other.handle_click(p.x + 5, p.y + PICKER_ROW + 2, CW, CH, false);
        other.refresh(&vfs);
        assert_eq!((other.canvas.width(), other.canvas.height()), (16, 8));
    }

    const PICKER_ROW: i32 = layout::PICKER_ROW_H as i32;

    #[test]
    fn shortcuts_new_undo_grid() {
        let vfs = make_vfs();
        let mut app = PaintApp::new("/apps/paint");
        click_canvas_pixel(&mut app, 1, 1);
        assert_eq!(app.undo_stack.len(), 1);
        app.handle_key(&Key::Char('z'), Modifiers::CTRL, &vfs);
        assert_eq!(app.canvas.get_pixel(1, 1), Color::rgb(255, 255, 255));
        app.handle_key(&Key::Char('y'), Modifiers::CTRL, &vfs);
        assert_eq!(app.canvas.get_pixel(1, 1), app.color);
        app.handle_key(&Key::Char('g'), Modifiers::NONE, &vfs);
        assert!(app.show_grid);
        app.handle_key(&Key::Char(']'), Modifiers::NONE, &vfs);
        assert_eq!(app.brush_size, 2);
        app.handle_key(&Key::Char('n'), Modifiers::CTRL, &vfs);
        assert!(app.undo_stack.is_empty());
        assert_eq!(app.canvas.get_pixel(1, 1), Color::rgb(255, 255, 255));
        // Plain letters other than shortcuts fall through.
        assert!(
            app.handle_key(&Key::Char('q'), Modifiers::NONE, &vfs)
                .is_none()
        );
    }

    #[test]
    fn menu_actions_layers_and_brush() {
        let mut app = PaintApp::new("/apps/paint");
        app.run_menu_action("layer.add");
        assert_eq!(app.canvas.layer_count(), 2);
        assert_eq!(app.canvas.active_layer(), 1);
        app.run_menu_action("layer.next");
        assert_eq!(app.canvas.active_layer(), 0);
        app.run_menu_action("brush.size.4");
        assert_eq!(app.brush_size, 4);
        app.run_menu_action("tool.7");
        assert_eq!(app.tool, Tool::Fill);
        app.run_menu_action("brush.color.prev");
        assert_eq!(app.palette_index, 15);
        assert_eq!(app.run_menu_action("file.exit"), AppAction::Exit);
    }

    #[test]
    fn menu_click_opens_dropdown() {
        let mut app = PaintApp::new("/apps/paint");
        // "File" label sits at the left of the bar.
        app.handle_click(12, 8, CW, CH, false);
        assert_eq!(app.menu.open, Some(0));
        // Clicking the canvas while open just closes the menu.
        click_canvas_pixel(&mut app, 0, 0);
        assert!(!app.menu.is_open());
        assert!(app.undo_stack.is_empty());
    }

    #[test]
    fn open_rejects_bad_file() {
        let mut vfs = make_vfs();
        io::ensure_dir(&mut vfs, PICTURES_DIR).expect("mkdir");
        vfs.write("/home/user/pictures/bad.bmp", b"garbage")
            .expect("write");
        let mut app = PaintApp::new("/apps/paint");
        assert!(
            app.open_from_vfs(&vfs, "/home/user/pictures/bad.bmp")
                .is_err()
        );
        assert_eq!(app.canvas.width(), CANVAS_W);
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
