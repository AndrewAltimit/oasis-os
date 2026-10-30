//! Body rows of the list / text categories (everything but Appearance).

use super::*;

impl SettingsApp {
    /// Rows for the Display category.
    pub(super) fn display_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(tr!("settings.skin_selection").to_string()));
        rows.push(Row::Blank);
        for (i, name) in builtin_names().iter().enumerate() {
            rows.push(Row::Item {
                item: i,
                label: (*name).to_string(),
                active: *name == self.current_skin,
            });
        }
        rows.push(Row::Blank);
        rows.push(Row::Text(tr!(
            "settings.resolution_value",
            width = self.width,
            height = self.height,
        )));
    }

    /// Rows for the Resolution category.
    pub(super) fn resolution_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(tr!("settings.virtual_resolution").to_string()));
        rows.push(Row::Blank);
        for (i, (w, h)) in RESOLUTION_PRESETS.iter().enumerate() {
            rows.push(Row::Item {
                item: i,
                label: format!("{w}x{h}  {}", preset_label(*w, *h)),
                active: *w == self.width && *h == self.height,
            });
        }
        rows.push(Row::Blank);
        rows.push(Row::Text(tr!("settings.resolution_live").to_string()));
    }

    /// Rows for the Audio category.
    pub(super) fn audio_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(tr!("settings.audio_title").to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Slider {
            item: 0,
            label: tr!("system.volume").to_string(),
            value: self.volume as f32,
            min: 0.0,
            max: 100.0,
            value_text: format!("{}%", self.volume),
        });
        rows.push(Row::Blank);
        rows.push(Row::Text(tr!("settings.volume_hint").to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Text(tr!("settings.audio_output").to_string()));
        rows.push(Row::Text(tr!("settings.sample_rate").to_string()));
        rows.push(Row::Text(tr!("settings.channels").to_string()));
    }

    /// Rows for the Language category.
    pub(super) fn language_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(tr!("settings.language_title").to_string()));
        rows.push(Row::Blank);
        for (i, locale) in Locale::all().iter().enumerate() {
            rows.push(Row::Item {
                item: i,
                label: locale_label(*locale),
                active: *locale == self.locale,
            });
        }
        rows.push(Row::Blank);
        if !oasis_i18n::bitmap_font_supports(self.locale) {
            rows.push(Row::Text(tr!("settings.language_fallback").to_string()));
        }
    }

    /// Rows for the Accessibility category.
    pub(super) fn accessibility_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(
            tr!("settings.accessibility_title").to_string(),
        ));
        rows.push(Row::Blank);
        let (min, max) = font_scale_range();
        rows.push(Row::Slider {
            item: A11Y_FONT_SCALE,
            label: tr!("settings.font_scale").to_string(),
            value: self.font_scale,
            min,
            max,
            value_text: format!("{:.0}%", self.font_scale * 100.0),
        });
        rows.push(Row::Toggle {
            item: A11Y_HIGH_CONTRAST,
            label: tr!("settings.high_contrast").to_string(),
            on: self.high_contrast(),
        });
        rows.push(Row::Toggle {
            item: A11Y_REDUCED_MOTION,
            label: tr!("settings.reduced_motion").to_string(),
            on: self.reduced_motion,
        });
        rows.push(Row::Blank);
        rows.push(Row::Text(tr!("settings.a11y_hint").to_string()));
    }

    /// Rows for the System category.
    pub(super) fn system_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(tr!("settings.system_title").to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Text(format!("Backend:       {}", self.backend_name)));
        rows.push(Row::Text(format!(
            "Resolution:    {} x {}",
            self.width, self.height
        )));
        rows.push(Row::Text(format!("Active Skin:   {}", self.current_skin)));
        rows.push(Row::Text(format!(
            "Version:       {}",
            env!("CARGO_PKG_VERSION")
        )));
        rows.push(Row::Blank);
        rows.push(Row::Text("VFS:           MemoryVfs".to_string()));
        rows.push(Row::Text("Rust Edition:  2024".to_string()));
        rows.push(Row::Text("MSRV:          1.91.0".to_string()));
    }

    /// Rows for the About category.
    pub(super) fn about_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(tr!("settings.about_title").to_string()));
        rows.push(Row::Blank);
        rows.push(Row::Text(format!(
            "Version:    {}",
            env!("CARGO_PKG_VERSION")
        )));
        for text in [
            "License:    MIT / Unlicense",
            "Crates:     20 workspace crates",
            "Apps:       16 built-in",
            "Skins:      18 built-in",
        ] {
            rows.push(Row::Text(text.to_string()));
        }
        rows.push(Row::Blank);
        for text in [
            "An embeddable operating system",
            "framework originally ported from",
            "Inspired by PSP homebrew (PSIX).",
        ] {
            rows.push(Row::Text(text.to_string()));
        }
        rows.push(Row::Blank);
        rows.push(Row::Text("github.com/AndrewAltimit/oasis-os".to_string()));
    }
}

/// List label for a locale: its native name when the bitmap font can draw
/// it, otherwise the English name plus the code.
pub(super) fn locale_label(locale: Locale) -> String {
    if oasis_i18n::bitmap_font_supports(locale) {
        locale.name().to_string()
    } else {
        format!("{} ({})", locale.english_name(), locale.code())
    }
}

/// Friendly label for a resolution preset (aspect ratio / common name).
pub(super) fn preset_label(w: u32, h: u32) -> &'static str {
    match (w, h) {
        (480, 272) => "(PSP)",
        (800, 600) => "(4:3 SVGA)",
        (1024, 768) => "(4:3 XGA)",
        (1280, 720) => "(16:9 HD)",
        (1600, 900) => "(16:9 HD+)",
        (1920, 1080) => "(16:9 FHD)",
        _ => "",
    }
}
