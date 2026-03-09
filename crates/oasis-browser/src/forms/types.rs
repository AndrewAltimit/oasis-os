//! Form element types, submission types, and keyboard input types.

// -----------------------------------------------------------------------
// Form element types
// -----------------------------------------------------------------------

/// The input type for a text-like `<input>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputType {
    /// Plain text input.
    Text,
    /// Password input (value masked on display).
    Password,
    /// Email address input.
    Email,
    /// Numeric input.
    Number,
    /// Hidden input (not displayed, not focusable).
    Hidden,
}

/// A single option inside a `<select>` dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    /// The value submitted with the form.
    pub value: String,
    /// The display label shown to the user.
    pub label: String,
    /// Whether this option is disabled.
    pub disabled: bool,
}

/// A form element that can appear inside a `<form>`.
#[derive(Debug, Clone, PartialEq)]
pub enum FormElement {
    /// A single-line text input field.
    TextInput {
        name: String,
        value: String,
        placeholder: String,
        maxlength: Option<usize>,
        input_type: InputType,
    },
    /// A checkbox.
    Checkbox {
        name: String,
        value: String,
        checked: bool,
        label: String,
    },
    /// A radio button belonging to a named group.
    RadioButton {
        name: String,
        value: String,
        checked: bool,
        group: String,
    },
    /// A dropdown select box.
    SelectBox {
        name: String,
        options: Vec<SelectOption>,
        selected_index: Option<usize>,
    },
    /// A multi-line text area.
    TextArea {
        name: String,
        value: String,
        rows: u32,
        cols: u32,
        placeholder: String,
    },
    /// A submit button.
    SubmitButton {
        name: String,
        value: String,
        label: String,
    },
    /// A reset button.
    ResetButton { label: String },
    /// A hidden input (not displayed, not focusable).
    HiddenInput { name: String, value: String },
}

impl FormElement {
    /// Returns the element name, if any.
    pub(super) fn name(&self) -> Option<&str> {
        match self {
            Self::TextInput { name, .. }
            | Self::Checkbox { name, .. }
            | Self::RadioButton { name, .. }
            | Self::SelectBox { name, .. }
            | Self::TextArea { name, .. }
            | Self::SubmitButton { name, .. }
            | Self::HiddenInput { name, .. } => {
                if name.is_empty() {
                    None
                } else {
                    Some(name)
                }
            },
            Self::ResetButton { .. } => None,
        }
    }

    /// Whether this element is focusable via tab navigation.
    pub(super) fn is_focusable(&self) -> bool {
        !matches!(self, Self::HiddenInput { .. })
    }
}

// -----------------------------------------------------------------------
// Form submission types
// -----------------------------------------------------------------------

/// HTTP method for form submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMethod {
    /// Submit via query string (`GET`).
    Get,
    /// Submit via request body (`POST`).
    Post,
}

/// Collected form data ready for submission.
#[derive(Debug, Clone, PartialEq)]
pub struct FormData {
    /// Key-value pairs of form field names and values.
    pub fields: Vec<(String, String)>,
    /// The HTTP method for submission.
    pub method: FormMethod,
    /// The action URL.
    pub action: String,
}

impl FormData {
    /// URL-encode the form data into an
    /// `application/x-www-form-urlencoded` string.
    pub fn encode(&self) -> String {
        self.fields
            .iter()
            .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }

    /// Return the fields as key-value pairs.
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        self.fields.clone()
    }
}

/// Minimal percent-encoding for form data (space -> `+`, special
/// chars -> `%XX`).
pub(super) fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            },
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0x0F));
            },
        }
    }
    out
}

/// Convert a nibble (0..15) to an uppercase hex digit.
fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + nibble - 10) as char,
    }
}

// -----------------------------------------------------------------------
// Keyboard input
// -----------------------------------------------------------------------

/// A key event relevant to form interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormKey {
    /// A printable character.
    Char(char),
    /// Backspace (delete before cursor).
    Backspace,
    /// Delete (delete after cursor).
    Delete,
    /// Enter / Return.
    Enter,
    /// Tab (focus next element).
    Tab,
    /// Shift+Tab (focus previous element).
    ShiftTab,
    /// Left arrow (move cursor left).
    Left,
    /// Right arrow (move cursor right).
    Right,
    /// Up arrow (select box: previous option).
    Up,
    /// Down arrow (select box: next option).
    Down,
    /// Home (move cursor to start).
    Home,
    /// End (move cursor to end).
    End,
    /// Space (toggle checkbox / select radio).
    Space,
}

/// The result of handling a form key event.
#[derive(Debug, Clone, PartialEq)]
pub enum FormAction {
    /// Nothing happened.
    None,
    /// A form was submitted.
    Submit(FormData),
    /// Focus moved to a different element.
    FocusChanged,
    /// A form value changed.
    ValueChanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- FormElement construction -----------------------------------

    #[test]
    fn text_input_construction() {
        let elem = FormElement::TextInput {
            name: "user".into(),
            value: "alice".into(),
            placeholder: "Enter name".into(),
            maxlength: Some(20),
            input_type: InputType::Text,
        };
        assert_eq!(elem.name(), Some("user"));
        assert!(elem.is_focusable());
    }

    #[test]
    fn password_input_type() {
        let elem = FormElement::TextInput {
            name: "pw".into(),
            value: String::new(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Password,
        };
        assert_eq!(elem.name(), Some("pw"));
        assert!(elem.is_focusable());
    }

    #[test]
    fn hidden_input_not_focusable() {
        let elem = FormElement::HiddenInput {
            name: "csrf".into(),
            value: "tok123".into(),
        };
        assert_eq!(elem.name(), Some("csrf"));
        assert!(!elem.is_focusable());
    }

    #[test]
    fn checkbox_construction() {
        let elem = FormElement::Checkbox {
            name: "agree".into(),
            value: "yes".into(),
            checked: true,
            label: "I agree".into(),
        };
        assert_eq!(elem.name(), Some("agree"));
        assert!(elem.is_focusable());
    }

    #[test]
    fn radio_button_construction() {
        let elem = FormElement::RadioButton {
            name: "color".into(),
            value: "red".into(),
            checked: false,
            group: "colors".into(),
        };
        assert_eq!(elem.name(), Some("color"));
        assert!(elem.is_focusable());
    }

    #[test]
    fn select_box_construction() {
        let opts = vec![
            SelectOption {
                value: "a".into(),
                label: "Alpha".into(),
                disabled: false,
            },
            SelectOption {
                value: "b".into(),
                label: "Beta".into(),
                disabled: true,
            },
        ];
        let elem = FormElement::SelectBox {
            name: "choice".into(),
            options: opts,
            selected_index: Some(0),
        };
        assert_eq!(elem.name(), Some("choice"));
        assert!(elem.is_focusable());
    }

    #[test]
    fn textarea_construction() {
        let elem = FormElement::TextArea {
            name: "bio".into(),
            value: "Hello".into(),
            rows: 5,
            cols: 40,
            placeholder: "Tell us about yourself".into(),
        };
        assert_eq!(elem.name(), Some("bio"));
        assert!(elem.is_focusable());
    }

    #[test]
    fn submit_button_construction() {
        let elem = FormElement::SubmitButton {
            name: "go".into(),
            value: "submit".into(),
            label: "Go".into(),
        };
        assert_eq!(elem.name(), Some("go"));
        assert!(elem.is_focusable());
    }

    #[test]
    fn reset_button_has_no_name() {
        let elem = FormElement::ResetButton {
            label: "Reset".into(),
        };
        assert_eq!(elem.name(), None);
        assert!(elem.is_focusable());
    }

    // -- empty name returns None ------------------------------------

    #[test]
    fn empty_name_returns_none() {
        let elem = FormElement::TextInput {
            name: String::new(),
            value: String::new(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
        };
        assert_eq!(elem.name(), None);
    }

    // -- InputType equality -----------------------------------------

    #[test]
    fn input_type_variants_distinct() {
        assert_ne!(InputType::Text, InputType::Password);
        assert_ne!(InputType::Email, InputType::Number);
        assert_ne!(InputType::Number, InputType::Hidden);
        assert_eq!(InputType::Text, InputType::Text);
    }

    #[test]
    fn input_type_debug() {
        assert_eq!(format!("{:?}", InputType::Email), "Email");
        assert_eq!(format!("{:?}", InputType::Password), "Password");
    }

    // -- FormMethod -------------------------------------------------

    #[test]
    fn form_method_equality() {
        assert_eq!(FormMethod::Get, FormMethod::Get);
        assert_eq!(FormMethod::Post, FormMethod::Post);
        assert_ne!(FormMethod::Get, FormMethod::Post);
    }

    #[test]
    fn form_method_debug() {
        assert_eq!(format!("{:?}", FormMethod::Get), "Get");
        assert_eq!(format!("{:?}", FormMethod::Post), "Post");
    }

    #[test]
    fn form_method_is_copy() {
        let m = FormMethod::Get;
        let m2 = m; // Copy
        assert_eq!(m, m2);
    }

    // -- FormData ---------------------------------------------------

    #[test]
    fn form_data_encode_empty() {
        let fd = FormData {
            fields: vec![],
            method: FormMethod::Get,
            action: "/search".into(),
        };
        assert_eq!(fd.encode(), "");
    }

    #[test]
    fn form_data_encode_single() {
        let fd = FormData {
            fields: vec![("q".into(), "hello".into())],
            method: FormMethod::Get,
            action: "/search".into(),
        };
        assert_eq!(fd.encode(), "q=hello");
    }

    #[test]
    fn form_data_encode_multiple() {
        let fd = FormData {
            fields: vec![("a".into(), "1".into()), ("b".into(), "2".into())],
            method: FormMethod::Post,
            action: "/submit".into(),
        };
        assert_eq!(fd.encode(), "a=1&b=2");
    }

    #[test]
    fn form_data_encode_spaces_as_plus() {
        let fd = FormData {
            fields: vec![("q".into(), "hello world".into())],
            method: FormMethod::Get,
            action: "/".into(),
        };
        assert_eq!(fd.encode(), "q=hello+world");
    }

    #[test]
    fn form_data_encode_special_chars() {
        let fd = FormData {
            fields: vec![("k".into(), "a&b=c".into())],
            method: FormMethod::Get,
            action: "/".into(),
        };
        assert_eq!(fd.encode(), "k=a%26b%3Dc");
    }

    #[test]
    fn form_data_to_pairs() {
        let fd = FormData {
            fields: vec![("x".into(), "1".into()), ("y".into(), "2".into())],
            method: FormMethod::Post,
            action: "/".into(),
        };
        let pairs = fd.to_pairs();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("x".into(), "1".into()));
        assert_eq!(pairs[1], ("y".into(), "2".into()));
    }

    // -- FormAction construction ------------------------------------

    #[test]
    fn form_action_none() {
        let a = FormAction::None;
        assert_eq!(a, FormAction::None);
    }

    #[test]
    fn form_action_submit() {
        let fd = FormData {
            fields: vec![("q".into(), "test".into())],
            method: FormMethod::Get,
            action: "/search".into(),
        };
        let a = FormAction::Submit(fd.clone());
        assert_eq!(a, FormAction::Submit(fd));
    }

    #[test]
    fn form_action_focus_changed() {
        assert_eq!(FormAction::FocusChanged, FormAction::FocusChanged);
        assert_ne!(FormAction::FocusChanged, FormAction::None);
    }

    #[test]
    fn form_action_value_changed() {
        assert_eq!(FormAction::ValueChanged, FormAction::ValueChanged);
        assert_ne!(FormAction::ValueChanged, FormAction::FocusChanged);
    }

    // -- url_encode -------------------------------------------------

    #[test]
    fn url_encode_unreserved_chars() {
        assert_eq!(url_encode("abc-XYZ_0.9~"), "abc-XYZ_0.9~");
    }

    #[test]
    fn url_encode_spaces() {
        assert_eq!(url_encode("a b c"), "a+b+c");
    }

    #[test]
    fn url_encode_special() {
        assert_eq!(url_encode("@!"), "%40%21");
    }

    #[test]
    fn url_encode_empty() {
        assert_eq!(url_encode(""), "");
    }

    // -- FormKey / SelectOption Debug --------------------------------

    #[test]
    fn form_key_debug() {
        assert_eq!(format!("{:?}", FormKey::Char('a')), "Char('a')");
        assert_eq!(format!("{:?}", FormKey::Backspace), "Backspace");
        assert_eq!(format!("{:?}", FormKey::Tab), "Tab");
    }

    #[test]
    fn form_key_equality() {
        assert_eq!(FormKey::Enter, FormKey::Enter);
        assert_eq!(FormKey::Char('x'), FormKey::Char('x'));
        assert_ne!(FormKey::Char('x'), FormKey::Char('y'));
        assert_ne!(FormKey::Tab, FormKey::ShiftTab);
    }

    #[test]
    fn select_option_equality() {
        let a = SelectOption {
            value: "v".into(),
            label: "L".into(),
            disabled: false,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn form_element_clone_eq() {
        let elem = FormElement::Checkbox {
            name: "c".into(),
            value: "on".into(),
            checked: true,
            label: "Check".into(),
        };
        let cloned = elem.clone();
        assert_eq!(elem, cloned);
    }
}
