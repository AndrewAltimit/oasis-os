//! Form data collection and serialization helpers.

use super::types::FormElement;

/// Convert a char-index cursor position to a byte offset.
pub(super) fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Collect form data from a slice of elements (used internally when
/// we only have a shared reference to the elements vec).
pub(super) fn collect_from_elements(elements: &[FormElement]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for elem in elements {
        match elem {
            FormElement::TextInput { name, value, .. } if !name.is_empty() => {
                pairs.push((name.clone(), value.clone()));
            },
            FormElement::Checkbox {
                name,
                value,
                checked,
                ..
            } if *checked && !name.is_empty() => {
                pairs.push((name.clone(), value.clone()));
            },
            FormElement::RadioButton {
                name,
                value,
                checked,
                ..
            } if *checked && !name.is_empty() => {
                pairs.push((name.clone(), value.clone()));
            },
            FormElement::SelectBox {
                name,
                options,
                selected_index,
            } if !name.is_empty() => {
                if let Some(idx) = selected_index
                    && let Some(opt) = options.get(*idx)
                {
                    pairs.push((name.clone(), opt.value.clone()));
                }
            },
            FormElement::TextArea { name, value, .. } if !name.is_empty() => {
                pairs.push((name.clone(), value.clone()));
            },
            FormElement::HiddenInput { name, value } if !name.is_empty() => {
                pairs.push((name.clone(), value.clone()));
            },
            _ => {},
        }
    }
    pairs
}
