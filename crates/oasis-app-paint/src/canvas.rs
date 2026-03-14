use oasis_types::backend::Color;

/// Default canvas width in pixels.
pub(crate) const CANVAS_W: u32 = 64;
/// Default canvas height in pixels.
pub(crate) const CANVAS_H: u32 = 48;
/// Maximum undo history depth.
pub(crate) const MAX_UNDO: usize = 20;

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

/// Multi-layer pixel canvas.
#[derive(Debug, Clone)]
pub struct Canvas {
    width: u32,
    height: u32,
    pub(crate) pixels: Vec<Color>,
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
pub(crate) fn alpha_blend(src: Color, dst: Color, layer_alpha: u8) -> Color {
    let effective_src = Color::rgba(
        src.r,
        src.g,
        src.b,
        (src.a as u32 * layer_alpha as u32 / 255) as u8,
    );
    effective_src.alpha_over(dst)
}

/// Encode pixel data as a 32-bit BMP file (BGRA, bottom-up).
pub(crate) fn encode_bmp(pixels: &[Color], w: u32, h: u32) -> Vec<u8> {
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
    pub(crate) layer: usize,
    pub(crate) snapshot: Vec<Color>,
}
