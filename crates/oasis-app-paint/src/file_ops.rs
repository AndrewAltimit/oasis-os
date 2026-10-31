//! Paint file I/O: Save / Save As to BMP, Open from a file or the
//! picker of `PICTURES_DIR`.

use super::*;

impl PaintApp {
    /// Encode the flattened canvas and write it to `path`, creating the
    /// parent folder if needed.
    pub(super) fn write_bmp(&mut self, vfs: &mut dyn Vfs, path: &str) -> Result<(), String> {
        let w = self.canvas.width();
        let h = self.canvas.height();
        let bmp = encode_bmp(&self.canvas.flatten(), w, h);
        oasis_app_core::vfs_util::write_creating_dirs(vfs, path, &bmp)
            .map_err(|e| e.to_string())?;
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

    /// Record the outcome of a file operation in the status line.
    pub(super) fn report(&mut self, result: Result<String, String>) {
        self.status = Some(match result {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        });
    }

    /// Open the File > Open picker (listing happens once a VFS is at hand).
    pub(super) fn begin_open(&mut self) {
        self.stop_drawing();
        self.picker = Some(Picker::default());
    }

    /// Fill in the picker listing and perform a click-chosen load. Needs
    /// only read access, so it runs from the input handlers and
    /// `apply_vfs_ops` alike.
    pub(super) fn service_picker(&mut self, vfs: &dyn Vfs) -> bool {
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
    pub(super) fn handle_picker_input(&mut self, button: &Button, vfs: &dyn Vfs, rows: usize) {
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
}
