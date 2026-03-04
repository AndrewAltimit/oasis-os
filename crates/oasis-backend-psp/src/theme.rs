//! Theme constants (matching oasis-core/src/theme.rs).

use oasis_backend_psp::{Color, SCREEN_HEIGHT, SCREEN_WIDTH};

// Bar geometry.
pub(crate) const STATUSBAR_H: u32 = 18;
pub(crate) const BOTTOMBAR_H: u32 = 32;
pub(crate) const BOTTOMBAR_Y: i32 = (SCREEN_HEIGHT - BOTTOMBAR_H) as i32;
pub(crate) const CONTENT_TOP: u32 = STATUSBAR_H;
pub(crate) const CONTENT_H: u32 = SCREEN_HEIGHT - CONTENT_TOP - BOTTOMBAR_H;

// Two-layer bottom bar row constants.
pub(crate) const BOTTOM_UPPER_Y: i32 = BOTTOMBAR_Y;
pub(crate) const BOTTOM_UPPER_H: u32 = 16;
pub(crate) const BOTTOM_LOWER_Y: i32 = BOTTOMBAR_Y + BOTTOM_UPPER_H as i32;

// Font metrics.
pub(crate) const CHAR_W: i32 = 8;

// Bottom bar layout.
pub(crate) const R_HINT_W: i32 = 28;

// Icon theme (compact to fit 4 rows).
pub(crate) const ICON_W: u32 = 42;
pub(crate) const ICON_H: u32 = 40;
pub(crate) const ICON_STRIPE_H: u32 = 8;
pub(crate) const ICON_FOLD_SIZE: u32 = 7;
pub(crate) const ICON_GFX_H: u32 = 16;
pub(crate) const ICON_GFX_PAD: u32 = 3;
pub(crate) const ICON_LABEL_PAD: i32 = 1;

// Dashboard grid (3 columns, 4 rows = 12 icons per page, L/R pagination).
pub(crate) const GRID_COLS: usize = 3;
pub(crate) const GRID_ROWS: usize = 4;
pub(crate) const GRID_PAD_X: i32 = 15;
pub(crate) const GRID_PAD_Y: i32 = 2;
pub(crate) const CELL_W: i32 = 150;
pub(crate) const CELL_H: i32 = (CONTENT_H as i32 - 2 * GRID_PAD_Y) / GRID_ROWS as i32;
pub(crate) const ICONS_PER_PAGE: usize = GRID_COLS * GRID_ROWS;
pub(crate) const CURSOR_PAD: i32 = 3;

// Persistent configuration path on Memory Stick.
pub(crate) const CONFIG_PATH: &str = "ms0:/PSP/GAME/OASISOS/config.rcfg";

// Colors -- bar backgrounds (green-tinted opaque, matching PSIX reference).
pub(crate) const STATUSBAR_BG: Color = Color::rgba(30, 80, 30, 200);
pub(crate) const BAR_BG: Color = Color::rgba(30, 80, 30, 200);
pub(crate) const SEPARATOR: Color = Color::rgba(180, 220, 180, 80);

// Colors -- status bar.
pub(crate) const BATTERY_CLR: Color = Color::rgb(120, 255, 120);
// Colors -- bottom bar.
pub(crate) const URL_CLR: Color = Color::rgb(200, 200, 200);
pub(crate) const USB_CLR: Color = Color::rgb(140, 140, 140);
pub(crate) const R_HINT_CLR: Color = Color::rgba(255, 255, 255, 140);
// Colors -- visualizer & transport.
pub(crate) const VIZ_BAR_PEAK: Color = Color::rgba(180, 100, 220, 230);
pub(crate) const TRANSPORT_CLR: Color = Color::rgba(220, 220, 220, 200);
pub(crate) const TRANSPORT_ACTIVE: Color = Color::rgb(120, 255, 120);
pub(crate) const L_HINT_CLR: Color = Color::rgba(255, 255, 255, 140);

// Visualizer constants.
pub(crate) const VIZ_BAR_COUNT: i32 = 14;
pub(crate) const VIZ_BAR_W: i32 = 3;
pub(crate) const VIZ_BAR_GAP: i32 = 1;
pub(crate) const VIZ_BAR_MAX_H: i32 = 12;
pub(crate) const VIZ_BAR_MIN_H: i32 = 1;

// Colors -- chrome bezel (green-tinted, matching PSIX reference).
pub(crate) const BEZEL_FILL: Color = Color::rgba(50, 100, 50, 120);
pub(crate) const BEZEL_TOP: Color = Color::rgba(200, 240, 200, 140);
pub(crate) const BEZEL_BOTTOM: Color = Color::rgba(20, 50, 20, 160);
pub(crate) const BEZEL_LEFT: Color = Color::rgba(180, 220, 180, 100);
pub(crate) const BEZEL_RIGHT: Color = Color::rgba(30, 60, 30, 140);

// Colors -- icons.
pub(crate) const BODY_CLR: Color = Color::rgb(250, 250, 248);
pub(crate) const FOLD_CLR: Color = Color::rgb(210, 210, 205);
pub(crate) const OUTLINE_CLR: Color = Color::rgba(255, 255, 255, 180);
pub(crate) const SHADOW_CLR: Color = Color::rgba(0, 0, 0, 70);
pub(crate) const LABEL_CLR: Color = Color::rgba(255, 255, 255, 230);

// Icon graphic symbol colors.
pub(crate) const ICON_SYM_CLR: Color = Color::rgba(255, 255, 255, 200);

// Label shadow.
pub(crate) const LABEL_SHADOW: Color = Color::rgba(0, 0, 0, 120);

// Button hints.
pub(crate) const HINT_BG: Color = Color::rgba(0, 0, 0, 120);
pub(crate) const HINT_BTN_CLR: Color = Color::rgb(200, 200, 100);
pub(crate) const HINT_TEXT_CLR: Color = Color::rgb(180, 180, 180);
pub(crate) const HINT_Y_OFFSET: i32 = 10;

// Terminal.
pub(crate) const MAX_OUTPUT_LINES: usize = 20;
pub(crate) const TERM_INPUT_Y: i32 = BOTTOMBAR_Y - 14;

// File manager.
pub(crate) const FM_VISIBLE_ROWS: usize = 18;
pub(crate) const FM_ROW_H: i32 = 10;
pub(crate) const FM_START_Y: i32 = CONTENT_TOP as i32 + 14;
