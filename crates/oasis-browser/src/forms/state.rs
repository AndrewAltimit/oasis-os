//! Per-form state tracking.

use super::types::{FormElement, FormMethod};

// -----------------------------------------------------------------------
// FormState -- per-form state
// -----------------------------------------------------------------------

/// State for a single `<form>` element, including all its child
/// elements and their current values.
#[derive(Debug, Clone)]
pub struct FormState {
    /// Unique identifier for this form.
    pub form_id: usize,
    /// The action URL for submission.
    pub action: String,
    /// The HTTP method for submission.
    pub method: FormMethod,
    /// Ordered list of form elements.
    pub(super) elements: Vec<FormElement>,
    /// Default values for each element, used by [`super::FormManager::reset`].
    pub(super) defaults: Vec<FormElement>,
    /// Cursor position within the currently-focused text field.
    pub(super) cursor: usize,
}

impl FormState {
    /// Create a new empty form.
    pub(super) fn new(form_id: usize, action: String, method: FormMethod) -> Self {
        Self {
            form_id,
            action,
            method,
            elements: Vec::new(),
            defaults: Vec::new(),
            cursor: 0,
        }
    }

    /// Add an element to this form. A snapshot is kept as default.
    pub(super) fn add_element(&mut self, element: FormElement) {
        self.defaults.push(element.clone());
        self.elements.push(element);
    }

    /// Names of all focusable elements, in order.
    pub(super) fn focusable_names(&self) -> Vec<String> {
        self.elements
            .iter()
            .filter(|e| e.is_focusable())
            .filter_map(|e| match e {
                FormElement::ResetButton { .. } => Some("__reset__".to_string()),
                FormElement::SubmitButton { name, .. } => {
                    if name.is_empty() {
                        Some("__submit__".to_string())
                    } else {
                        Some(name.clone())
                    }
                },
                other => other.name().map(String::from),
            })
            .collect()
    }

    /// Find element index by name.
    pub(super) fn index_of(&self, name: &str) -> Option<usize> {
        self.elements.iter().position(|e| match e {
            FormElement::ResetButton { .. } => name == "__reset__",
            FormElement::SubmitButton { name: n, .. } => {
                if n.is_empty() {
                    name == "__submit__"
                } else {
                    n == name
                }
            },
            _ => e.name() == Some(name),
        })
    }

    /// Collect form data for submission.
    pub(super) fn collect(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for elem in &self.elements {
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

    /// Reset all elements to their default values.
    pub(super) fn reset(&mut self) {
        self.elements = self.defaults.clone();
        self.cursor = 0;
    }
}

// -----------------------------------------------------------------------
// ElementKind (borrow-safe dispatch tag)
// -----------------------------------------------------------------------

/// Lightweight tag extracted from a [`FormElement`] to allow
/// dispatching in [`super::FormManager::handle_input`] without holding
/// a mutable borrow on the element across the entire match.
pub(super) enum ElementKind {
    TextInput { maxlength: Option<usize> },
    TextArea,
    Checkbox,
    RadioButton { group: String, value: String },
    SelectBox,
    SubmitButton,
    ResetButton,
    Hidden,
}

impl ElementKind {
    pub(super) fn of(elem: &FormElement) -> Self {
        match elem {
            FormElement::TextInput { maxlength, .. } => Self::TextInput {
                maxlength: *maxlength,
            },
            FormElement::TextArea { .. } => Self::TextArea,
            FormElement::Checkbox { .. } => Self::Checkbox,
            FormElement::RadioButton { group, value, .. } => Self::RadioButton {
                group: group.clone(),
                value: value.clone(),
            },
            FormElement::SelectBox { .. } => Self::SelectBox,
            FormElement::SubmitButton { .. } => Self::SubmitButton,
            FormElement::ResetButton { .. } => Self::ResetButton,
            FormElement::HiddenInput { .. } => Self::Hidden,
        }
    }
}
