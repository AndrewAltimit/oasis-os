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
