//! Icon atlas rendering.
//!
//! A single 256x256 texture containing 16x16 icons in a 16x16 grid provides
//! all UI symbols. Icons are grayscale with alpha; theme colors are applied
//! via `blit_sub_tinted`.

use oasis_types::backend::{Color, SdiBackend, TextureId};
use oasis_types::error::Result;

/// Well-known icon identifiers.
///
/// Each maps to a position in the icon atlas texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Icon {
    // Navigation
    Back = 0,
    Forward,
    Home,
    Refresh,
    Close,
    Minimize,
    Maximize,
    // Actions
    Search,
    Settings,
    Menu,
    MoreVertical,
    MoreHorizontal,
    Edit,
    Delete,
    Copy,
    Paste,
    Save,
    Download,
    Upload,
    Share,
    // Status
    CheckCircle,
    Warning,
    Error,
    Info,
    Help,
    Lock,
    Unlock,
    Eye,
    EyeOff,
    Bell,
    BellOff,
    // Media
    Play,
    Pause,
    Stop,
    SkipForward,
    SkipBack,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    Repeat,
    Shuffle,
    // Content
    File,
    Folder,
    FolderOpen,
    Image,
    Document,
    Code,
    Terminal,
    Globe,
    Link,
    ExternalLink,
    Mail,
    Chat,
    // Arrows and indicators
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Plus,
    Minus,
    Star,
    StarFilled,
    Heart,
    HeartFilled,
    // System
    Wifi,
    WifiOff,
    Battery,
    BatteryLow,
    Clock,
    Calendar,
    User,
    Users,
    Cpu,
    Memory,
}

/// Renderer for an icon atlas texture.
pub struct IconAtlas {
    pub texture: TextureId,
    pub icon_size: u32,
    pub cols: u32,
}

impl IconAtlas {
    /// Create a new atlas renderer.
    ///
    /// `icon_size` is typically 16 for a 256x256 atlas (16 columns).
    pub fn new(texture: TextureId, icon_size: u32, cols: u32) -> Self {
        Self {
            texture,
            icon_size,
            cols,
        }
    }

    /// Draw an icon at native size with a tint color.
    pub fn draw(
        &self,
        backend: &mut dyn SdiBackend,
        icon: Icon,
        x: i32,
        y: i32,
        tint: Color,
    ) -> Result<()> {
        let idx = icon as u32;
        let col = idx % self.cols;
        let row = idx / self.cols;
        let sx = col * self.icon_size;
        let sy = row * self.icon_size;
        backend.blit_sub_tinted(
            self.texture,
            sx,
            sy,
            self.icon_size,
            self.icon_size,
            x,
            y,
            self.icon_size,
            self.icon_size,
            tint,
        )
    }

    /// Draw an icon scaled to a specific size.
    pub fn draw_scaled(
        &self,
        backend: &mut dyn SdiBackend,
        icon: Icon,
        x: i32,
        y: i32,
        size: u32,
        tint: Color,
    ) -> Result<()> {
        let idx = icon as u32;
        let col = idx % self.cols;
        let row = idx / self.cols;
        let sx = col * self.icon_size;
        let sy = row * self.icon_size;
        backend.blit_sub_tinted(
            self.texture,
            sx,
            sy,
            self.icon_size,
            self.icon_size,
            x,
            y,
            size,
            size,
            tint,
        )
    }
}
