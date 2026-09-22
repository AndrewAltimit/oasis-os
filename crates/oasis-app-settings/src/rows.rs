//! Row model for the Settings body.
//!
//! Every category builds a list of [`Row`]s. The windowed renderer draws
//! them with oasis-ui widgets (sliders, toggles, highlighted list rows);
//! [`Row::to_line`] gives the plain-text rendition used for the fullscreen
//! SDI path and for [`oasis_app_core::App::lines`].

/// One row of the Settings body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Row {
    /// Empty spacer row.
    Blank,
    /// Section heading.
    Heading(String),
    /// Non-interactive information text.
    Text(String),
    /// Selectable list entry; `active` marks the currently applied value.
    Item {
        item: usize,
        label: String,
        active: bool,
    },
    /// Numeric control drawn as an oasis-ui `Slider`.
    Slider {
        item: usize,
        label: String,
        value: f32,
        min: f32,
        max: f32,
        value_text: String,
    },
    /// On/off control drawn as an oasis-ui `Toggle`.
    Toggle {
        item: usize,
        label: String,
        on: bool,
    },
}

impl Row {
    /// Selectable item index of this row, if it is interactive.
    pub(crate) fn item(&self) -> Option<usize> {
        match self {
            Row::Item { item, .. } | Row::Slider { item, .. } | Row::Toggle { item, .. } => {
                Some(*item)
            },
            _ => None,
        }
    }

    /// Plain-text rendition (fullscreen SDI path, `lines()`).
    pub(crate) fn to_line(&self) -> String {
        match self {
            Row::Blank => String::new(),
            Row::Heading(s) | Row::Text(s) => format!("  {s}"),
            Row::Item { label, active, .. } => {
                let marker = if *active { " *" } else { "" };
                format!("   {label}{marker}")
            },
            Row::Slider {
                label,
                value,
                min,
                max,
                value_text,
                ..
            } => {
                let span = (max - min).max(f32::EPSILON);
                let frac = ((value - min) / span).clamp(0.0, 1.0);
                let filled = (frac * 20.0).round() as usize;
                format!(
                    "   {label}: [{}{}] {value_text}",
                    "\u{2588}".repeat(filled),
                    "\u{2591}".repeat(20 - filled),
                )
            },
            Row::Toggle { label, on, .. } => {
                let key = if *on { "ui.on" } else { "ui.off" };
                let state = oasis_i18n::tr!(key).to_uppercase();
                format!("   {label}: [{state}]")
            },
        }
    }
}

/// Row index of the given selectable item, if present.
pub(crate) fn row_of_item(rows: &[Row], item: usize) -> Option<usize> {
    rows.iter().position(|r| r.item() == Some(item))
}

/// First visible row so that `focus_row` (if any) is on screen, or the
/// clamped free-scroll offset for text-only categories.
pub(crate) fn scroll_for(
    rows: usize,
    visible: usize,
    focus_row: Option<usize>,
    free_scroll: usize,
) -> usize {
    let max_scroll = rows.saturating_sub(visible);
    match focus_row {
        // Keep one row of context below the focus when possible.
        Some(r) => (r + 2).saturating_sub(visible).min(max_scroll),
        None => free_scroll.min(max_scroll),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_line_has_bar_and_value() {
        let row = Row::Slider {
            item: 0,
            label: "Volume".to_string(),
            value: 50.0,
            min: 0.0,
            max: 100.0,
            value_text: "50%".to_string(),
        };
        let line = row.to_line();
        assert!(line.contains("Volume"));
        assert!(line.contains("50%"));
        assert_eq!(line.matches('\u{2588}').count(), 10);
    }

    #[test]
    fn toggle_and_item_lines() {
        let t = Row::Toggle {
            item: 1,
            label: "Reduced Motion".to_string(),
            on: true,
        };
        assert!(t.to_line().contains("[ON]"));
        let i = Row::Item {
            item: 0,
            label: "classic".to_string(),
            active: true,
        };
        assert!(i.to_line().ends_with(" *"));
        assert_eq!(t.item(), Some(1));
        assert_eq!(Row::Blank.item(), None);
    }

    #[test]
    fn scroll_keeps_focus_visible() {
        assert_eq!(scroll_for(30, 10, Some(3), 0), 0);
        let s = scroll_for(30, 10, Some(20), 0);
        assert!(s <= 20 && 20 < s + 10);
        assert_eq!(scroll_for(30, 10, Some(29), 0), 20);
        assert_eq!(scroll_for(5, 10, None, 7), 0);
        assert_eq!(scroll_for(30, 10, None, 7), 7);
    }
}
