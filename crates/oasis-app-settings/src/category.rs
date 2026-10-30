//! The Settings categories (the tab strip).

use oasis_i18n::tr;

/// Settings categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Category {
    Display,
    Appearance,
    Resolution,
    Audio,
    Language,
    Accessibility,
    System,
    About,
}

impl Category {
    pub(super) const ALL: [Category; 8] = [
        Category::Display,
        Category::Appearance,
        Category::Resolution,
        Category::Audio,
        Category::Language,
        Category::Accessibility,
        Category::System,
        Category::About,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Category::Display => tr!("settings.cat_display"),
            Category::Appearance => tr!("settings.cat_appearance"),
            Category::Resolution => tr!("settings.cat_resolution"),
            Category::Audio => tr!("settings.cat_audio"),
            Category::Language => tr!("settings.cat_language"),
            Category::Accessibility => tr!("settings.cat_accessibility"),
            Category::System => tr!("settings.cat_system"),
            Category::About => tr!("settings.cat_about"),
        }
    }

    /// Whether Up/Down move a selection cursor over items (as opposed to
    /// adjusting a value or scrolling text).
    pub(super) fn has_items(self) -> bool {
        !matches!(self, Category::Audio | Category::System | Category::About)
    }
}
