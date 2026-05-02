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

    /// Check whether this form contains an element with the given name.
    pub fn has_element(&self, name: &str) -> bool {
        self.index_of(name).is_some()
    }

    /// Read-only view of this form's elements.
    ///
    /// Exposed so outer layers (`BrowserWidget`) can synchronise
    /// in-flight form state back onto the DOM without reaching into
    /// module-private fields.
    pub fn elements(&self) -> &[FormElement] {
        &self.elements
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
                    ..
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
#[derive(Debug)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forms::types::{InputType, SelectOption};

    // -- helpers ----------------------------------------------------

    fn text_input(name: &str, value: &str) -> FormElement {
        FormElement::TextInput {
            name: name.into(),
            value: value.into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: None,
            min: None,
            max: None,
        }
    }

    fn hidden(name: &str, value: &str) -> FormElement {
        FormElement::HiddenInput {
            name: name.into(),
            value: value.into(),
        }
    }

    fn checkbox(name: &str, value: &str, checked: bool) -> FormElement {
        FormElement::Checkbox {
            name: name.into(),
            value: value.into(),
            checked,
            label: String::new(),
        }
    }

    fn radio(name: &str, value: &str, group: &str, checked: bool) -> FormElement {
        FormElement::RadioButton {
            name: name.into(),
            value: value.into(),
            checked,
            group: group.into(),
        }
    }

    fn select_box(name: &str, values: &[&str], sel: Option<usize>) -> FormElement {
        let options = values
            .iter()
            .map(|v| SelectOption {
                value: (*v).into(),
                label: (*v).into(),
                disabled: false,
            })
            .collect();
        FormElement::SelectBox {
            name: name.into(),
            options,
            selected_index: sel,
            open: false,
        }
    }

    fn submit(name: &str) -> FormElement {
        FormElement::SubmitButton {
            name: name.into(),
            value: "submit".into(),
            label: "Submit".into(),
        }
    }

    fn reset() -> FormElement {
        FormElement::ResetButton {
            label: "Reset".into(),
        }
    }

    // -- FormState::new ---------------------------------------------

    #[test]
    fn new_form_is_empty() {
        let f = FormState::new(1, "/action".into(), FormMethod::Get);
        assert_eq!(f.form_id, 1);
        assert_eq!(f.action, "/action");
        assert_eq!(f.method, FormMethod::Get);
        assert!(f.elements.is_empty());
        assert!(f.defaults.is_empty());
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn new_form_post_method() {
        let f = FormState::new(42, "/submit".into(), FormMethod::Post);
        assert_eq!(f.method, FormMethod::Post);
    }

    // -- add_element ------------------------------------------------

    #[test]
    fn add_element_stores_and_snapshots() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("q", ""));
        assert_eq!(f.elements.len(), 1);
        assert_eq!(f.defaults.len(), 1);
        assert_eq!(f.elements[0], f.defaults[0]);
    }

    #[test]
    fn add_multiple_elements() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("a", ""));
        f.add_element(text_input("b", ""));
        f.add_element(hidden("c", "tok"));
        assert_eq!(f.elements.len(), 3);
        assert_eq!(f.defaults.len(), 3);
    }

    // -- focusable_names --------------------------------------------

    #[test]
    fn focusable_names_skips_hidden() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("q", ""));
        f.add_element(hidden("csrf", "tok"));
        f.add_element(submit("go"));
        let names = f.focusable_names();
        assert_eq!(names, vec!["q", "go"]);
    }

    #[test]
    fn focusable_names_submit_unnamed() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(submit("")); // unnamed submit
        let names = f.focusable_names();
        assert_eq!(names, vec!["__submit__"]);
    }

    #[test]
    fn focusable_names_reset_button() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(reset());
        let names = f.focusable_names();
        assert_eq!(names, vec!["__reset__"]);
    }

    #[test]
    fn focusable_names_all_types() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("t", ""));
        f.add_element(checkbox("cb", "on", false));
        f.add_element(radio("r", "v", "grp", false));
        f.add_element(select_box("sel", &["a", "b"], Some(0)));
        f.add_element(FormElement::TextArea {
            name: "ta".into(),
            value: String::new(),
            rows: 3,
            cols: 40,
            placeholder: String::new(),
            required: false,
            minlength: None,
            maxlength: None,
        });
        f.add_element(hidden("h", "x"));
        f.add_element(submit("go"));
        f.add_element(reset());
        let names = f.focusable_names();
        assert_eq!(names, vec!["t", "cb", "r", "sel", "ta", "go", "__reset__"]);
    }

    // -- index_of ---------------------------------------------------

    #[test]
    fn index_of_existing() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("a", ""));
        f.add_element(text_input("b", ""));
        assert_eq!(f.index_of("a"), Some(0));
        assert_eq!(f.index_of("b"), Some(1));
    }

    #[test]
    fn index_of_missing() {
        let f = FormState::new(0, "/".into(), FormMethod::Get);
        assert_eq!(f.index_of("nope"), None);
    }

    #[test]
    fn index_of_submit_unnamed() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(submit(""));
        assert_eq!(f.index_of("__submit__"), Some(0));
    }

    #[test]
    fn index_of_submit_named() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(submit("go"));
        assert_eq!(f.index_of("go"), Some(0));
        assert_eq!(f.index_of("__submit__"), None);
    }

    #[test]
    fn index_of_reset() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(reset());
        assert_eq!(f.index_of("__reset__"), Some(0));
    }

    // -- collect ----------------------------------------------------

    #[test]
    fn collect_text_inputs() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("q", "rust"));
        f.add_element(text_input("lang", "en"));
        let data = f.collect();
        assert_eq!(
            data,
            vec![("q".into(), "rust".into()), ("lang".into(), "en".into()),]
        );
    }

    #[test]
    fn collect_skips_empty_names() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("", "orphan"));
        assert!(f.collect().is_empty());
    }

    #[test]
    fn collect_checkbox_only_if_checked() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(checkbox("agree", "yes", true));
        f.add_element(checkbox("extra", "yes", false));
        let data = f.collect();
        assert_eq!(data, vec![("agree".into(), "yes".into())]);
    }

    #[test]
    fn collect_radio_only_if_checked() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(radio("color", "red", "c", false));
        f.add_element(radio("color", "blue", "c", true));
        let data = f.collect();
        assert_eq!(data, vec![("color".into(), "blue".into())]);
    }

    #[test]
    fn collect_select_box() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(select_box("s", &["x", "y", "z"], Some(1)));
        let data = f.collect();
        assert_eq!(data, vec![("s".into(), "y".into())]);
    }

    #[test]
    fn collect_select_box_none_selected() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(select_box("s", &["x"], None));
        assert!(f.collect().is_empty());
    }

    #[test]
    fn collect_textarea() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(FormElement::TextArea {
            name: "bio".into(),
            value: "Hello world".into(),
            rows: 3,
            cols: 40,
            placeholder: String::new(),
            required: false,
            minlength: None,
            maxlength: None,
        });
        let data = f.collect();
        assert_eq!(data, vec![("bio".into(), "Hello world".into())]);
    }

    #[test]
    fn collect_hidden_input() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(hidden("csrf", "abc123"));
        let data = f.collect();
        assert_eq!(data, vec![("csrf".into(), "abc123".into())]);
    }

    #[test]
    fn collect_skips_buttons() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(submit("go"));
        f.add_element(reset());
        assert!(f.collect().is_empty());
    }

    // -- reset ------------------------------------------------------

    #[test]
    fn reset_restores_defaults() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("q", "initial"));
        // Mutate current value
        f.elements[0] = text_input("q", "changed");
        f.cursor = 5;
        assert_eq!(f.elements[0], text_input("q", "changed"));

        f.reset();
        assert_eq!(f.elements[0], text_input("q", "initial"));
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn reset_preserves_element_count() {
        let mut f = FormState::new(0, "/".into(), FormMethod::Get);
        f.add_element(text_input("a", "1"));
        f.add_element(text_input("b", "2"));
        f.reset();
        assert_eq!(f.elements.len(), 2);
    }

    // -- ElementKind::of --------------------------------------------

    #[test]
    fn element_kind_text_input() {
        let elem = FormElement::TextInput {
            name: "q".into(),
            value: String::new(),
            placeholder: String::new(),
            maxlength: Some(100),
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: None,
            min: None,
            max: None,
        };
        let kind = ElementKind::of(&elem);
        assert!(
            matches!(
                kind,
                ElementKind::TextInput {
                    maxlength: Some(100),
                }
            ),
            "expected TextInput kind, got {kind:?}"
        );
    }

    #[test]
    fn element_kind_text_input_no_max() {
        let elem = text_input("q", "");
        let __out = ElementKind::of(&elem);
        assert!(
            matches!(&__out, ElementKind::TextInput { .. }),
            "expected TextInput with no maxlength, got {__out:?}"
        );
    }

    #[test]
    fn element_kind_textarea() {
        let elem = FormElement::TextArea {
            name: "t".into(),
            value: String::new(),
            rows: 3,
            cols: 40,
            placeholder: String::new(),
            required: false,
            minlength: None,
            maxlength: None,
        };
        assert!(matches!(ElementKind::of(&elem), ElementKind::TextArea));
    }

    #[test]
    fn element_kind_checkbox() {
        let elem = checkbox("c", "on", true);
        assert!(matches!(ElementKind::of(&elem), ElementKind::Checkbox));
    }

    #[test]
    fn element_kind_radio_button() {
        let elem = radio("color", "red", "colors", true);
        let __out = ElementKind::of(&elem);
        let ElementKind::RadioButton { group, value } = __out else {
            panic!("expected RadioButton kind, got {__out:?}");
        };
        assert_eq!(group, "colors");
        assert_eq!(value, "red");
    }

    #[test]
    fn element_kind_select_box() {
        let elem = select_box("s", &["a"], Some(0));
        assert!(matches!(ElementKind::of(&elem), ElementKind::SelectBox));
    }

    #[test]
    fn element_kind_submit() {
        let elem = submit("go");
        assert!(matches!(ElementKind::of(&elem), ElementKind::SubmitButton));
    }

    #[test]
    fn element_kind_reset() {
        let elem = reset();
        assert!(matches!(ElementKind::of(&elem), ElementKind::ResetButton));
    }

    #[test]
    fn element_kind_hidden() {
        let elem = hidden("h", "v");
        assert!(matches!(ElementKind::of(&elem), ElementKind::Hidden));
    }
}
