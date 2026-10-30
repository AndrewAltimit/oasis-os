//! Skin inheritance: `inherits = "<parent>"` fills unset child fields.

use super::Skin;

impl Skin {
    /// Merge missing theme fields from a parent skin.
    ///
    /// Only fills in fields that are `None` / empty in the child. Explicitly
    /// set fields in the child always win.
    pub fn merge_theme_from(&mut self, parent: &Skin) {
        let pt = &parent.theme;
        let ct = &mut self.theme;

        // Only override extended optional fields when the child hasn't set them.
        if ct.surface.is_none() {
            ct.surface.clone_from(&pt.surface);
        }
        if ct.accent.is_none() {
            ct.accent.clone_from(&pt.accent);
        }
        if ct.accent_hover.is_none() {
            ct.accent_hover.clone_from(&pt.accent_hover);
        }
        if ct.success.is_none() {
            ct.success.clone_from(&pt.success);
        }
        if ct.warning.is_none() {
            ct.warning.clone_from(&pt.warning);
        }
        if ct.border_radius.is_none() {
            ct.border_radius = pt.border_radius;
        }
        if ct.shadow_intensity.is_none() {
            ct.shadow_intensity = pt.shadow_intensity;
        }
        if ct.gradient_enabled.is_none() {
            ct.gradient_enabled = pt.gradient_enabled;
        }
        if ct.wm_theme.is_none() {
            ct.wm_theme.clone_from(&pt.wm_theme);
        }
        if ct.bar_overrides.is_none() {
            ct.bar_overrides.clone_from(&pt.bar_overrides);
        }
        if ct.icon_overrides.is_none() {
            ct.icon_overrides.clone_from(&pt.icon_overrides);
        }
        if ct.browser_overrides.is_none() {
            ct.browser_overrides.clone_from(&pt.browser_overrides);
        }
        if ct.app_overrides.is_none() {
            ct.app_overrides.clone_from(&pt.app_overrides);
        }
        if ct.osk_overrides.is_none() {
            ct.osk_overrides.clone_from(&pt.osk_overrides);
        }
        if ct.start_menu_overrides.is_none() {
            ct.start_menu_overrides.clone_from(&pt.start_menu_overrides);
        }
        if ct.wallpaper.is_none() {
            ct.wallpaper.clone_from(&pt.wallpaper);
        }
        if ct.cursor.is_none() {
            ct.cursor.clone_from(&pt.cursor);
        }
        if ct.background_layers.is_none() {
            ct.background_layers.clone_from(&pt.background_layers);
        }
        if ct.chrome_layers.is_none() {
            ct.chrome_layers.clone_from(&pt.chrome_layers);
        }
        if ct.geometry.is_none() {
            ct.geometry.clone_from(&pt.geometry);
        }
        if ct.typography.is_none() {
            ct.typography.clone_from(&pt.typography);
        }
        if ct.transition.is_none() {
            ct.transition.clone_from(&pt.transition);
        }
        if ct.scrollbar_overrides.is_none() {
            ct.scrollbar_overrides.clone_from(&pt.scrollbar_overrides);
        }
        if ct.palette.is_none() {
            ct.palette.clone_from(&pt.palette);
        }
        if ct.boot.is_none() {
            ct.boot.clone_from(&pt.boot);
        }
        // Merge collection fields: only fill if child has none.
        if ct.app_themes.is_none() {
            ct.app_themes.clone_from(&pt.app_themes);
        }
        if ct.gradients.is_none() {
            ct.gradients.clone_from(&pt.gradients);
        }
        if ct.animations.is_none() {
            ct.animations.clone_from(&pt.animations);
        }
        if ct.widget_states.is_none() {
            ct.widget_states.clone_from(&pt.widget_states);
        }

        // Merge layout: parent objects fill in missing child objects.
        for (name, def) in &parent.layout.objects {
            if !self.layout.objects.contains_key(name) {
                self.layout.objects.insert(name.clone(), def.clone());
            }
        }

        // Merge assets: parent images fill in missing child assets so
        // inherited layout objects keep their textures.
        for (name, asset) in &parent.assets {
            if !self.assets.contains_key(name) {
                self.assets.insert(name.clone(), asset.clone());
            }
        }

        // Same for font assets, so an inherited `[typography] font` resolves.
        for (name, bytes) in &parent.font_assets {
            if !self.font_assets.contains_key(name) {
                self.font_assets.insert(name.clone(), bytes.clone());
            }
        }

        // Merge UI sounds the same way: the parent's [sounds] table fills
        // in when the child has none, and parent WAVs back inherited paths.
        if ct.sounds.is_none() {
            ct.sounds.clone_from(&pt.sounds);
        }
        for (name, bytes) in &parent.sound_assets {
            if !self.sound_assets.contains_key(name) {
                self.sound_assets.insert(name.clone(), bytes.clone());
            }
        }
    }
}
