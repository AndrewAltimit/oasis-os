//! The Appearance category: the 9 base-color palette editor (live
//! contrast readout, apply / save as a custom skin, variants).

use super::*;

/// Labels for the 9 editable base palette colors, in `SkinTheme` order.
pub(super) const BASE_COLOR_LABELS: [&str; 9] = [
    "Background",
    "Primary",
    "Secondary",
    "Text",
    "Dim Text",
    "Status Bar",
    "Prompt",
    "Output",
    "Error",
];

/// Number of action rows after the 9 color rows in the Appearance list
/// (Apply, Save, and one row per variant).
pub(super) const APPEARANCE_ACTIONS: usize = 2 + SkinVariant::ALL.len();

/// For each base-color role (indexed like [`BASE_COLOR_LABELS`]), the partner
/// role its contrast is measured against and the WCAG AA ratio it should
/// clear. Surfaces (`Background`, `Status Bar`) are judged by the readability
/// of `Text` drawn on them; foreground roles are judged against `Background`.
/// `Text` itself is held to the 4.5:1 body-text minimum; the rest to 3.0:1.
pub(super) const CONTRAST_PARTNERS: [(usize, f64); 9] = [
    (3, 4.5), // Background   vs Text
    (0, 3.0), // Primary      vs Background
    (0, 3.0), // Secondary    vs Background
    (0, 4.5), // Text         vs Background
    (0, 3.0), // Dim Text     vs Background
    (3, 4.5), // Status Bar   vs Text
    (0, 3.0), // Prompt       vs Background
    (0, 3.0), // Output       vs Background
    (0, 3.0), // Error        vs Background
];

/// Format a color as `#RRGGBB`.
pub(super) fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Short label for a base-color role, used in the inline contrast readout.
pub(super) fn short_role(role: usize) -> &'static str {
    match role {
        0 => "Bg",
        3 => "Text",
        _ => BASE_COLOR_LABELS[role],
    }
}

/// State for the Appearance base-color editor.
#[derive(Debug)]
pub(super) struct AppearanceState {
    /// The 9 editable base palette colors (order matches
    /// [`BASE_COLOR_LABELS`]).
    pub(super) colors: [Color; 9],
    /// Which skin the palette was loaded from (reloaded when the shell
    /// publishes a different active skin).
    pub(super) loaded_for: String,
    /// Active channel while editing a color row (0 = R, 1 = G, 2 = B).
    /// `None` when not in edit mode.
    pub(super) editing_channel: Option<u8>,
    /// Color value before edit mode was entered, for Cancel-revert.
    pub(super) edit_backup: Color,
}

impl Default for AppearanceState {
    fn default() -> Self {
        Self {
            colors: [Color::BLACK; 9],
            loaded_for: String::new(),
            editing_channel: None,
            edit_backup: Color::BLACK,
        }
    }
}

impl AppearanceState {
    /// Load the palette from a skin's theme.
    pub(super) fn load_from_theme(&mut self, theme: &SkinTheme, skin_name: &str) {
        let parse = |s: &str, fallback: Color| parse_hex_color(s).unwrap_or(fallback);
        self.colors = [
            parse(&theme.background, Color::BLACK),
            parse(&theme.primary, Color::rgb(50, 100, 200)),
            parse(&theme.secondary, Color::rgb(80, 80, 80)),
            parse(&theme.text, Color::WHITE),
            parse(&theme.dim_text, Color::rgb(128, 128, 128)),
            parse(&theme.status_bar, Color::rgb(40, 60, 90)),
            parse(&theme.prompt, Color::rgb(0, 255, 0)),
            parse(&theme.output, Color::rgb(204, 204, 204)),
            parse(&theme.error, Color::rgb(255, 68, 68)),
        ];
        self.loaded_for = skin_name.to_string();
        self.editing_channel = None;
    }

    /// Write the palette into a theme's 9 base color fields.
    pub(super) fn write_to_theme(&self, theme: &mut SkinTheme) {
        theme.background = hex(self.colors[0]);
        theme.primary = hex(self.colors[1]);
        theme.secondary = hex(self.colors[2]);
        theme.text = hex(self.colors[3]);
        theme.dim_text = hex(self.colors[4]);
        theme.status_bar = hex(self.colors[5]);
        theme.prompt = hex(self.colors[6]);
        theme.output = hex(self.colors[7]);
        theme.error = hex(self.colors[8]);
    }
}

impl SettingsApp {
    /// Rows for the Appearance category (base-color editor).
    ///
    /// Items are the 9 base colors followed by the action rows (Apply /
    /// Save / variants), with no gaps so `item_cursor` maps 1:1 to rows.
    pub(super) fn appearance_rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Heading(tr!("settings.appearance_title").to_string()));
        rows.push(Row::Blank);

        for (i, label) in BASE_COLOR_LABELS.iter().enumerate() {
            let c = self.appearance.colors[i];
            let readout = self.contrast_readout(i);
            let text = if self.item_cursor == i && self.appearance.editing_channel.is_some() {
                let ch = self.appearance.editing_channel.unwrap_or(0);
                let mark = |idx: u8, name: char, v: u8| {
                    if ch == idx {
                        format!("[{name}:{v:3}]")
                    } else {
                        format!(" {name}:{v:3} ")
                    }
                };
                format!(
                    "{label:<11} {} {}{}{} {readout}",
                    hex(c),
                    mark(0, 'R', c.r),
                    mark(1, 'G', c.g),
                    mark(2, 'B', c.b),
                )
            } else {
                format!("{label:<11} {}  {readout}", hex(c))
            };
            rows.push(Row::Item {
                item: i,
                label: text,
                active: false,
            });
        }

        let n = BASE_COLOR_LABELS.len();
        let mut actions = vec![
            format!("[ {} ]", tr!("settings.apply_preview")),
            format!(
                "[ {} ]",
                tr!("settings.save_as", name = self.custom_skin_name())
            ),
        ];
        actions.extend(
            SkinVariant::ALL
                .iter()
                .map(|v| format!("[ {} ]", tr!("settings.variant", name = v.label()))),
        );
        for (i, label) in actions.into_iter().enumerate() {
            rows.push(Row::Item {
                item: n + i,
                label,
                active: false,
            });
        }

        rows.push(Row::Blank);
        if self.appearance.editing_channel.is_some() {
            rows.push(Row::Text(tr!("settings.edit_hint_value").to_string()));
            rows.push(Row::Text(tr!("settings.edit_hint_done").to_string()));
        } else {
            rows.push(Row::Text(tr!("settings.edit_hint").to_string()));
            rows.push(Row::Text(tr!("settings.contrast_hint").to_string()));
        }
    }

    /// Inline contrast readout for a base-color row: the ratio against the
    /// role's sensible partner color (see [`CONTRAST_PARTNERS`]) plus an
    /// `AA` / `low` verdict at the WCAG AA threshold. Recomputed on every
    /// refresh, so it updates live as a channel is stepped.
    pub(super) fn contrast_readout(&self, role: usize) -> String {
        let (partner, required) = CONTRAST_PARTNERS[role];
        let ratio = contrast_ratio(
            self.appearance.colors[role],
            self.appearance.colors[partner],
        );
        let verdict = if ratio >= required { "AA" } else { "low" };
        format!("vs {} {ratio:.1}:1 {verdict}", short_role(partner))
    }

    /// Name used by "Save as custom skin": `custom-<base>` where `<base>` is
    /// the current skin without any existing `custom-` prefix or variant
    /// suffix, so repeated saves don't stack prefixes.
    pub(super) fn custom_skin_name(&self) -> String {
        let base = self
            .current_skin
            .strip_prefix("custom-")
            .unwrap_or(&self.current_skin);
        format!("custom-{base}")
    }

    /// Ensure the Appearance palette reflects the currently active skin.
    pub(super) fn ensure_appearance_palette(&mut self) {
        if self.appearance.loaded_for == self.current_skin {
            return;
        }
        let name = self.current_skin.clone();
        match resolve_skin(&name) {
            Ok(skin) => self.appearance.load_from_theme(&skin.theme, &name),
            Err(_) => {
                let default_theme = SkinTheme::default();
                self.appearance.load_from_theme(&default_theme, &name);
            },
        }
    }

    /// The theme to apply/save: the active skin's theme with the edited
    /// palette written over its 9 base colors.
    pub(super) fn edited_theme(&self) -> SkinTheme {
        let mut theme = resolve_skin(&self.current_skin)
            .map(|s| s.theme)
            .unwrap_or_default();
        self.appearance.write_to_theme(&mut theme);
        theme
    }

    /// Handle Confirm on an Appearance row.
    pub(super) fn appearance_confirm(&mut self) {
        if self.item_cursor < BASE_COLOR_LABELS.len() {
            // Enter edit mode on a color row.
            self.appearance.editing_channel = Some(0);
            self.appearance.edit_backup = self.appearance.colors[self.item_cursor];
            self.refresh_lines();
            return;
        }
        let action = self.item_cursor - BASE_COLOR_LABELS.len();
        match action {
            // Apply (preview): send the edited theme for an in-memory swap.
            0 => {
                if let Ok(toml_doc) = self.edited_theme().to_toml_string() {
                    self.post(SKIN_APPLY_THEME_REQUEST_PATH, toml_doc);
                }
            },
            // Save as custom skin: the shell writes skins/<name>/ and swaps.
            1 => {
                if let Ok(toml_doc) = self.edited_theme().to_toml_string() {
                    let payload = format!("{}\n{toml_doc}", self.custom_skin_name());
                    self.post(SKIN_SAVE_CUSTOM_REQUEST_PATH, payload);
                }
            },
            // Variant rows: transform the local palette, then auto-preview so
            // the variant is immediately visible (and saveable afterwards).
            n => {
                if let Some(&variant) = SkinVariant::ALL.get(n - 2) {
                    let variant_theme = self.edited_theme().derive_variant(variant);
                    let loaded_for = self.appearance.loaded_for.clone();
                    self.appearance.load_from_theme(&variant_theme, &loaded_for);
                    if let Ok(toml_doc) = variant_theme.to_toml_string() {
                        self.post(SKIN_APPLY_THEME_REQUEST_PATH, toml_doc);
                    }
                    self.refresh_lines();
                }
            },
        }
    }

    /// Handle input while a color row is in edit mode. Returns the action to
    /// bubble up (always `None`; Cancel exits edit mode, not the app).
    pub(super) fn handle_appearance_edit(&mut self, button: &Button) -> AppAction {
        let Some(channel) = self.appearance.editing_channel else {
            return AppAction::None;
        };
        let idx = self.item_cursor.min(BASE_COLOR_LABELS.len() - 1);
        match button {
            Button::Left => {
                self.appearance.editing_channel = Some((channel + 2) % 3);
            },
            Button::Right => {
                self.appearance.editing_channel = Some((channel + 1) % 3);
            },
            Button::Up | Button::Down => {
                let c = &mut self.appearance.colors[idx];
                let field = match channel {
                    0 => &mut c.r,
                    1 => &mut c.g,
                    _ => &mut c.b,
                };
                const STEP: u8 = 8;
                *field = if matches!(button, Button::Up) {
                    field.saturating_add(STEP)
                } else {
                    field.saturating_sub(STEP)
                };
            },
            Button::Confirm | Button::Triangle => {
                self.appearance.editing_channel = None;
            },
            Button::Cancel => {
                self.appearance.colors[idx] = self.appearance.edit_backup;
                self.appearance.editing_channel = None;
            },
            _ => return AppAction::None,
        }
        self.refresh_lines();
        AppAction::None
    }
}
