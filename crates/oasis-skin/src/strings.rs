//! Skin strings -- user-facing text for menus, prompts, and messages.
//!
//! All displayed text is skin-configurable via `strings.toml`. This enables
//! different personas (military-style for Tactical, hacker-style for Terminal,
//! garbled for Corrupted) without code changes.

use serde::Deserialize;

/// User-facing text strings for a skin.
#[derive(Debug, Clone, Deserialize)]
pub struct SkinStrings {
    /// Boot sequence text lines (displayed during startup animation).
    #[serde(default)]
    pub boot_text: Vec<String>,
    /// Terminal prompt format. Use `{cwd}` as placeholder for current directory.
    #[serde(default = "default_prompt")]
    pub prompt_format: String,
    /// Display title shown in the status bar or window title.
    #[serde(default = "default_title")]
    pub title: String,
    /// Label for the "home" or main menu page.
    #[serde(default = "default_home_label")]
    pub home_label: String,
    /// Error prefix shown before command errors.
    #[serde(default = "default_error_prefix")]
    pub error_prefix: String,
    /// Shutdown message.
    #[serde(default = "default_shutdown")]
    pub shutdown_message: String,
    /// Welcome message shown after boot.
    #[serde(default = "default_welcome")]
    pub welcome_message: String,

    // -- Navigation labels --
    /// "Back" navigation label.
    #[serde(default = "default_back")]
    pub back: String,
    /// "Close" label (window/dialog close).
    #[serde(default = "default_close")]
    pub close: String,
    /// "OK" confirmation label.
    #[serde(default = "default_ok")]
    pub ok: String,
    /// "Cancel" label.
    #[serde(default = "default_cancel")]
    pub cancel: String,
    /// "Yes" label.
    #[serde(default = "default_yes")]
    pub yes: String,
    /// "No" label.
    #[serde(default = "default_no")]
    pub no: String,

    // -- App name labels --
    /// File Manager app name.
    #[serde(default = "default_file_manager")]
    pub file_manager: String,
    /// Settings app name.
    #[serde(default = "default_settings")]
    pub settings: String,
    /// Browser app name.
    #[serde(default = "default_browser")]
    pub browser: String,
    /// Music Player app name.
    #[serde(default = "default_music")]
    pub music: String,
    /// Network app name.
    #[serde(default = "default_network")]
    pub network: String,
    /// System Monitor app name.
    #[serde(default = "default_system_monitor")]
    pub system_monitor: String,
    /// Photo Viewer app name.
    #[serde(default = "default_photos")]
    pub photos: String,
    /// Package Manager app name.
    #[serde(default = "default_packages")]
    pub packages: String,
    /// TV Guide app name.
    #[serde(default = "default_tv_guide")]
    pub tv_guide: String,

    // -- Status labels --
    /// Battery status label.
    #[serde(default = "default_battery")]
    pub battery: String,
    /// Wi-Fi status label.
    #[serde(default = "default_wifi")]
    pub wifi: String,
}

fn default_prompt() -> String {
    "$> ".to_string()
}
fn default_title() -> String {
    "OASIS_OS".to_string()
}
fn default_home_label() -> String {
    "Home".to_string()
}
fn default_error_prefix() -> String {
    "error: ".to_string()
}
fn default_shutdown() -> String {
    "System halted.".to_string()
}
fn default_welcome() -> String {
    "Welcome to OASIS_OS.".to_string()
}
fn default_back() -> String {
    "Back".to_string()
}
fn default_close() -> String {
    "Close".to_string()
}
fn default_ok() -> String {
    "OK".to_string()
}
fn default_cancel() -> String {
    "Cancel".to_string()
}
fn default_yes() -> String {
    "Yes".to_string()
}
fn default_no() -> String {
    "No".to_string()
}
fn default_file_manager() -> String {
    "File Manager".to_string()
}
fn default_settings() -> String {
    "Settings".to_string()
}
fn default_browser() -> String {
    "Browser".to_string()
}
fn default_music() -> String {
    "Music".to_string()
}
fn default_network() -> String {
    "Network".to_string()
}
fn default_system_monitor() -> String {
    "System Monitor".to_string()
}
fn default_photos() -> String {
    "Photos".to_string()
}
fn default_packages() -> String {
    "Packages".to_string()
}
fn default_tv_guide() -> String {
    "TV Guide".to_string()
}
fn default_battery() -> String {
    "Battery".to_string()
}
fn default_wifi() -> String {
    "Wi-Fi".to_string()
}

impl Default for SkinStrings {
    fn default() -> Self {
        Self {
            boot_text: Vec::new(),
            prompt_format: default_prompt(),
            title: default_title(),
            home_label: default_home_label(),
            error_prefix: default_error_prefix(),
            shutdown_message: default_shutdown(),
            welcome_message: default_welcome(),
            back: default_back(),
            close: default_close(),
            ok: default_ok(),
            cancel: default_cancel(),
            yes: default_yes(),
            no: default_no(),
            file_manager: default_file_manager(),
            settings: default_settings(),
            browser: default_browser(),
            music: default_music(),
            network: default_network(),
            system_monitor: default_system_monitor(),
            photos: default_photos(),
            packages: default_packages(),
            tv_guide: default_tv_guide(),
            battery: default_battery(),
            wifi: default_wifi(),
        }
    }
}

impl SkinStrings {
    /// Format the prompt with the current working directory substituted.
    pub fn format_prompt(&self, cwd: &str) -> String {
        self.prompt_format.replace("{cwd}", cwd)
    }

    /// Look up a string by key name. Returns `None` for unknown keys.
    pub fn get(&self, key: &str) -> Option<&str> {
        match key {
            "title" => Some(&self.title),
            "home_label" => Some(&self.home_label),
            "error_prefix" => Some(&self.error_prefix),
            "shutdown_message" => Some(&self.shutdown_message),
            "welcome_message" => Some(&self.welcome_message),
            "prompt_format" => Some(&self.prompt_format),
            "back" => Some(&self.back),
            "close" => Some(&self.close),
            "ok" => Some(&self.ok),
            "cancel" => Some(&self.cancel),
            "yes" => Some(&self.yes),
            "no" => Some(&self.no),
            "file_manager" => Some(&self.file_manager),
            "settings" => Some(&self.settings),
            "browser" => Some(&self.browser),
            "music" => Some(&self.music),
            "network" => Some(&self.network),
            "system_monitor" => Some(&self.system_monitor),
            "photos" => Some(&self.photos),
            "packages" => Some(&self.packages),
            "tv_guide" => Some(&self.tv_guide),
            "battery" => Some(&self.battery),
            "wifi" => Some(&self.wifi),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_strings() {
        let s = SkinStrings::default();
        assert_eq!(s.prompt_format, "$> ");
        assert_eq!(s.title, "OASIS_OS");
        assert!(s.boot_text.is_empty());
    }

    #[test]
    fn default_navigation_strings() {
        let s = SkinStrings::default();
        assert_eq!(s.back, "Back");
        assert_eq!(s.close, "Close");
        assert_eq!(s.ok, "OK");
        assert_eq!(s.cancel, "Cancel");
        assert_eq!(s.yes, "Yes");
        assert_eq!(s.no, "No");
    }

    #[test]
    fn default_app_name_strings() {
        let s = SkinStrings::default();
        assert_eq!(s.file_manager, "File Manager");
        assert_eq!(s.settings, "Settings");
        assert_eq!(s.browser, "Browser");
        assert_eq!(s.music, "Music");
        assert_eq!(s.network, "Network");
        assert_eq!(s.system_monitor, "System Monitor");
        assert_eq!(s.photos, "Photos");
        assert_eq!(s.packages, "Packages");
        assert_eq!(s.tv_guide, "TV Guide");
    }

    #[test]
    fn default_status_strings() {
        let s = SkinStrings::default();
        assert_eq!(s.battery, "Battery");
        assert_eq!(s.wifi, "Wi-Fi");
    }

    #[test]
    fn format_prompt_substitution() {
        let s = SkinStrings {
            prompt_format: "{cwd} $ ".to_string(),
            ..SkinStrings::default()
        };
        assert_eq!(s.format_prompt("/home"), "/home $ ");
    }

    #[test]
    fn format_prompt_no_placeholder() {
        let s = SkinStrings {
            prompt_format: "root# ".to_string(),
            ..SkinStrings::default()
        };
        assert_eq!(s.format_prompt("/whatever"), "root# ");
    }

    #[test]
    fn deserialize_from_toml() {
        let toml = r#"
boot_text = ["Initializing...", "Loading modules...", "Ready."]
prompt_format = "root@oasis:{cwd}# "
title = "OASIS_OS"
welcome_message = "SYSTEM ONLINE"
"#;
        let s: SkinStrings = toml::from_str(toml).unwrap();
        assert_eq!(s.boot_text.len(), 3);
        assert_eq!(s.prompt_format, "root@oasis:{cwd}# ");
        assert_eq!(s.title, "OASIS_OS");
        assert_eq!(s.welcome_message, "SYSTEM ONLINE");
        // Defaults for unspecified fields.
        assert_eq!(s.error_prefix, "error: ");
        assert_eq!(s.back, "Back");
        assert_eq!(s.file_manager, "File Manager");
    }

    #[test]
    fn partial_deserialize_new_fields() {
        let toml = r#"
back = "Retour"
ok = "Valider"
file_manager = "Gestionnaire"
"#;
        let s: SkinStrings = toml::from_str(toml).unwrap();
        assert_eq!(s.back, "Retour");
        assert_eq!(s.ok, "Valider");
        assert_eq!(s.file_manager, "Gestionnaire");
        // Unspecified fields keep defaults.
        assert_eq!(s.title, "OASIS_OS");
        assert_eq!(s.close, "Close");
    }

    #[test]
    fn get_returns_known_keys() {
        let s = SkinStrings::default();
        assert_eq!(s.get("title"), Some("OASIS_OS"));
        assert_eq!(s.get("back"), Some("Back"));
        assert_eq!(s.get("file_manager"), Some("File Manager"));
        assert_eq!(s.get("battery"), Some("Battery"));
    }

    #[test]
    fn get_returns_none_for_unknown() {
        let s = SkinStrings::default();
        assert_eq!(s.get("nonexistent"), None);
        assert_eq!(s.get(""), None);
    }

    #[test]
    fn get_reflects_custom_values() {
        let s = SkinStrings {
            back: "戻る".to_string(),
            ok: "決定".to_string(),
            ..SkinStrings::default()
        };
        assert_eq!(s.get("back"), Some("戻る"));
        assert_eq!(s.get("ok"), Some("決定"));
    }

    #[test]
    fn deserialize_japanese_strings() {
        let toml = r#"
title = "OASIS_OS"
back = "戻る"
close = "閉じる"
ok = "決定"
cancel = "キャンセル"
yes = "はい"
no = "いいえ"
file_manager = "ファイル管理"
settings = "設定"
browser = "ブラウザ"
music = "音楽"
battery = "バッテリー"
wifi = "無線LAN"
"#;
        let s: SkinStrings = toml::from_str(toml).unwrap();
        assert_eq!(s.back, "戻る");
        assert_eq!(s.close, "閉じる");
        assert_eq!(s.file_manager, "ファイル管理");
        assert_eq!(s.wifi, "無線LAN");
    }
}
