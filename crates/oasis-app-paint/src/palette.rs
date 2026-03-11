use oasis_types::backend::Color;

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
pub(crate) const PALETTE_NAMES: [&str; 16] = [
    "Black", "White", "Red", "Green", "Blue", "Yellow", "Magenta", "Cyan", "Gray", "DkRed",
    "DkGreen", "DkBlue", "Olive", "Purple", "Teal", "Orange",
];
