//! Skin inheritance: `inherits = "<parent>"` fills unset child fields.

use std::collections::HashMap;

use crate::strings::SkinStrings;
use crate::theme::SkinTheme;

use super::Skin;

impl Skin {
    /// Merge missing theme fields from a parent skin.
    ///
    /// Only fills in fields the child left unset: `None` options, and --
    /// for fields with serde defaults (the nine base colors, `text_scale`,
    /// every `strings.toml` entry) -- keys absent from the child's own
    /// documents. Explicitly set fields in the child always win, even when
    /// they equal the default. Layout objects and assets merge by name;
    /// collection tables inherit whole-table only when the child has none.
    pub fn merge_theme_from(&mut self, parent: &Skin) {
        merge_theme(&mut self.theme, &parent.theme, &self.explicit_keys.theme);
        merge_strings(
            &mut self.strings,
            &parent.strings,
            &self.explicit_keys.strings,
        );
        // What the merged skin now carries counts as set for its own
        // children (a grandchild inherits the merged values).
        self.explicit_keys
            .theme
            .extend(parent.explicit_keys.theme.iter().cloned());
        self.explicit_keys
            .strings
            .extend(parent.explicit_keys.strings.iter().cloned());

        // Layout objects, image / font / sound assets: parent entries fill
        // in missing child entries so inherited references resolve.
        fill_map(&mut self.layout.objects, &parent.layout.objects);
        fill_map(&mut self.assets, &parent.assets);
        fill_map(&mut self.font_assets, &parent.font_assets);
        fill_map(&mut self.sound_assets, &parent.sound_assets);
    }
}

/// Insert every `parent` entry whose key `child` lacks.
fn fill_map<V: Clone>(child: &mut HashMap<String, V>, parent: &HashMap<String, V>) {
    for (name, value) in parent {
        if !child.contains_key(name) {
            child.insert(name.clone(), value.clone());
        }
    }
}

/// Fill the child's unset theme fields from the parent.
///
/// The destructuring is exhaustive on purpose: a new `SkinTheme` field
/// fails to compile here until someone decides how it inherits.
fn merge_theme(ct: &mut SkinTheme, pt: &SkinTheme, explicit: &std::collections::BTreeSet<String>) {
    let SkinTheme {
        background,
        primary,
        secondary,
        text,
        dim_text,
        status_bar,
        prompt,
        output,
        error,
        surface,
        accent,
        accent_hover,
        success,
        warning,
        border_radius,
        shadow_intensity,
        gradient_enabled,
        text_scale,
        density,
        elevation,
        wm_theme,
        bar_overrides,
        icon_overrides,
        browser_overrides,
        app_overrides,
        osk_overrides,
        start_menu_overrides,
        wallpaper,
        cursor,
        geometry,
        typography,
        transition,
        scrollbar_overrides,
        palette,
        boot,
        background_layers,
        chrome_layers,
        background_performance,
        app_themes,
        gradients,
        animations,
        widget_states,
        sounds,
    } = ct;

    // Defaulted fields: inherit only when the child's theme.toml omits them.
    macro_rules! defaulted {
        ($($field:ident),* $(,)?) => {$(
            if !explicit.contains(stringify!($field)) {
                $field.clone_from(&pt.$field);
            }
        )*};
    }
    defaulted!(
        background, primary, secondary, text, dim_text, status_bar, prompt, output, error,
        text_scale,
    );

    // Optional fields (whole tables included): inherit when unset.
    macro_rules! optional {
        ($($field:ident),* $(,)?) => {$(
            if $field.is_none() {
                $field.clone_from(&pt.$field);
            }
        )*};
    }
    optional!(
        surface,
        accent,
        accent_hover,
        success,
        warning,
        border_radius,
        shadow_intensity,
        gradient_enabled,
        density,
        elevation,
        wm_theme,
        bar_overrides,
        icon_overrides,
        browser_overrides,
        app_overrides,
        osk_overrides,
        start_menu_overrides,
        wallpaper,
        cursor,
        geometry,
        typography,
        transition,
        scrollbar_overrides,
        palette,
        boot,
        background_layers,
        chrome_layers,
        background_performance,
        app_themes,
        gradients,
        animations,
        widget_states,
        sounds,
    );
}

/// Fill the strings the child's `strings.toml` omits from the parent
/// (exhaustive for the same reason as [`merge_theme`]).
fn merge_strings(
    cs: &mut SkinStrings,
    ps: &SkinStrings,
    explicit: &std::collections::BTreeSet<String>,
) {
    let SkinStrings {
        boot_text,
        prompt_format,
        title,
        home_label,
        error_prefix,
        shutdown_message,
        welcome_message,
        back,
        close,
        ok,
        cancel,
        yes,
        no,
        file_manager,
        settings,
        browser,
        music,
        network,
        system_monitor,
        photos,
        packages,
        tv_guide,
        battery,
        wifi,
    } = cs;
    macro_rules! defaulted {
        ($($field:ident),* $(,)?) => {$(
            if !explicit.contains(stringify!($field)) {
                $field.clone_from(&ps.$field);
            }
        )*};
    }
    defaulted!(
        boot_text,
        prompt_format,
        title,
        home_label,
        error_prefix,
        shutdown_message,
        welcome_message,
        back,
        close,
        ok,
        cancel,
        yes,
        no,
        file_manager,
        settings,
        browser,
        music,
        network,
        system_monitor,
        photos,
        packages,
        tv_guide,
        battery,
        wifi,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skin(theme: &str, strings: &str) -> Skin {
        Skin::from_toml_full("name = \"t\"", "", "", theme, strings).unwrap()
    }

    #[test]
    fn omitted_base_colors_come_from_the_parent() {
        let parent = skin(
            "background = \"#102030\"\ntext = \"#EEEEEE\"\ntext_scale = 1.5\n",
            "",
        );
        let mut child = skin("primary = \"#FF0000\"\n", "");
        child.merge_theme_from(&parent);
        assert_eq!(child.theme.background, "#102030");
        assert_eq!(child.theme.text, "#EEEEEE");
        assert_eq!(child.theme.primary, "#FF0000");
        assert!((child.theme.text_scale - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn explicit_default_values_still_win() {
        // The child sets the default background on purpose: keep it.
        let default_bg = SkinTheme::default().background;
        let parent = skin("background = \"#102030\"\n", "");
        let mut child = skin(&format!("background = \"{default_bg}\"\n"), "");
        child.merge_theme_from(&parent);
        assert_eq!(child.theme.background, default_bg);
    }

    #[test]
    fn previously_dropped_tables_inherit() {
        let parent = skin(
            "density = \"compact\"\n[elevation]\n[background_performance]\nmax_layers = 3\n",
            "",
        );
        let mut child = skin("", "");
        child.merge_theme_from(&parent);
        assert_eq!(child.theme.density.as_deref(), Some("compact"));
        assert!(child.theme.elevation.is_some());
        assert_eq!(
            child
                .theme
                .background_performance
                .as_ref()
                .and_then(|p| p.max_layers),
            Some(3)
        );
    }

    #[test]
    fn omitted_strings_come_from_the_parent() {
        let parent = skin("", "title = \"PARENT\"\nsettings = \"Prefs\"\n");
        let mut child = skin("", "title = \"CHILD\"\n");
        child.merge_theme_from(&parent);
        assert_eq!(child.strings.title, "CHILD");
        assert_eq!(child.strings.settings, "Prefs");
    }

    #[test]
    fn grandchild_inherits_merged_values() {
        let grandparent = skin("background = \"#010203\"\n", "");
        let mut parent = skin("text = \"#FAFAFA\"\n", "");
        parent.merge_theme_from(&grandparent);
        let mut child = skin("", "");
        child.merge_theme_from(&parent);
        assert_eq!(child.theme.background, "#010203");
        assert_eq!(child.theme.text, "#FAFAFA");
    }
}
