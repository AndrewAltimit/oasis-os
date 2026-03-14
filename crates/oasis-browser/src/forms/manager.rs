//! [`FormManager`] — manages all forms on a page.

use super::state::{ElementKind, FormState};
use super::types::{FormAction, FormData, FormElement, FormKey, FormMethod, SelectOption};

// -----------------------------------------------------------------------
// FormManager
// -----------------------------------------------------------------------

/// Manages all forms on a page, including focus state and input
/// handling.
pub struct FormManager {
    /// All forms on the current page.
    pub forms: Vec<FormState>,
    /// Which form currently has focus (index into `forms`).
    pub focused_form: Option<usize>,
    /// Name of the currently focused element within that form.
    pub focused_element: Option<String>,
}

impl FormManager {
    /// Create a new empty form manager.
    pub fn new() -> Self {
        Self {
            forms: Vec::new(),
            focused_form: None,
            focused_element: None,
        }
    }

    /// Register a new form and return its id.
    pub fn add_form(&mut self, action: &str, method: FormMethod) -> usize {
        let id = self.forms.len();
        self.forms
            .push(FormState::new(id, action.to_string(), method));
        id
    }

    /// Add an element to the form with the given id.
    pub fn add_element(&mut self, form_id: usize, element: FormElement) {
        if let Some(form) = self.forms.get_mut(form_id) {
            form.add_element(element);
        }
    }

    /// Set the value of a named text element.
    pub fn set_value(&mut self, form_id: usize, name: &str, value: &str) {
        let Some(form) = self.forms.get_mut(form_id) else {
            return;
        };
        for elem in &mut form.elements {
            match elem {
                FormElement::TextInput {
                    name: n,
                    value: v,
                    maxlength,
                    ..
                } if n == name => {
                    *v = if let Some(max) = maxlength {
                        value.chars().take(*max).collect()
                    } else {
                        value.to_string()
                    };
                    return;
                },
                FormElement::TextArea {
                    name: n, value: v, ..
                } if n == name => {
                    *v = value.to_string();
                    return;
                },
                _ => {},
            }
        }
    }

    /// Get the value of a named text element.
    pub fn get_value(&self, form_id: usize, name: &str) -> Option<&str> {
        let form = self.forms.get(form_id)?;
        for elem in &form.elements {
            match elem {
                FormElement::TextInput { name: n, value, .. } if n == name => return Some(value),
                FormElement::TextArea { name: n, value, .. } if n == name => return Some(value),
                FormElement::HiddenInput { name: n, value } if n == name => return Some(value),
                _ => {},
            }
        }
        None
    }

    /// Toggle a checkbox by name.
    pub fn toggle_checkbox(&mut self, form_id: usize, name: &str) {
        let Some(form) = self.forms.get_mut(form_id) else {
            return;
        };
        for elem in &mut form.elements {
            if let FormElement::Checkbox {
                name: n, checked, ..
            } = elem
                && n == name
            {
                *checked = !*checked;
                return;
            }
        }
    }

    /// Select a radio button within a group, deselecting the others.
    pub fn select_radio(&mut self, form_id: usize, group: &str, value: &str) {
        let Some(form) = self.forms.get_mut(form_id) else {
            return;
        };
        for elem in &mut form.elements {
            if let FormElement::RadioButton {
                group: g,
                value: v,
                checked,
                ..
            } = elem
                && g == group
            {
                *checked = v == value;
            }
        }
    }

    /// Select an option in a select box by index.
    pub fn select_option(&mut self, form_id: usize, name: &str, index: usize) {
        let Some(form) = self.forms.get_mut(form_id) else {
            return;
        };
        for elem in &mut form.elements {
            if let FormElement::SelectBox {
                name: n,
                options,
                selected_index,
            } = elem
                && n == name
                && index < options.len()
            {
                if let Some(opt) = options.get(index)
                    && !opt.disabled
                {
                    *selected_index = Some(index);
                }
                return;
            }
        }
    }

    /// Focus the next focusable element (tab order).
    pub fn focus_next(&mut self) {
        self.advance_focus(true);
    }

    /// Focus the previous focusable element (shift-tab order).
    pub fn focus_prev(&mut self) {
        self.advance_focus(false);
    }

    /// Move focus forward or backward across all forms.
    fn advance_focus(&mut self, forward: bool) {
        // Build a flat list of (form_idx, element_name).
        let flat: Vec<(usize, String)> = self
            .forms
            .iter()
            .enumerate()
            .flat_map(|(fi, f)| f.focusable_names().into_iter().map(move |name| (fi, name)))
            .collect();

        if flat.is_empty() {
            return;
        }

        // Find current position in flat list.
        let current_pos = match (&self.focused_form, &self.focused_element) {
            (Some(fi), Some(name)) => flat.iter().position(|(f, n)| f == fi && n == name),
            _ => None,
        };

        let next_pos = match current_pos {
            Some(pos) => {
                if forward {
                    (pos + 1) % flat.len()
                } else if pos == 0 {
                    flat.len() - 1
                } else {
                    pos - 1
                }
            },
            None => {
                if forward {
                    0
                } else {
                    flat.len() - 1
                }
            },
        };

        let (fi, ref name) = flat[next_pos];
        self.focused_form = Some(fi);
        self.focused_element = Some(name.clone());

        // Reset cursor to end of text for newly focused text fields.
        if let Some(form) = self.forms.get_mut(fi)
            && let Some(idx) = form.index_of(name)
        {
            match &form.elements[idx] {
                FormElement::TextInput { value, .. } => {
                    form.cursor = value.len();
                },
                FormElement::TextArea { value, .. } => {
                    form.cursor = value.len();
                },
                _ => {
                    form.cursor = 0;
                },
            }
        }
    }

    /// Handle a keyboard event on the currently focused element.
    ///
    /// Returns a [`FormAction`] describing what happened.
    pub fn handle_input(&mut self, key: FormKey) -> FormAction {
        // Tab navigation is handled regardless of focus state.
        match key {
            FormKey::Tab => {
                self.focus_next();
                return FormAction::FocusChanged;
            },
            FormKey::ShiftTab => {
                self.focus_prev();
                return FormAction::FocusChanged;
            },
            _ => {},
        }

        let (fi, name) = match (&self.focused_form, &self.focused_element) {
            (Some(fi), Some(name)) => (*fi, name.clone()),
            _ => return FormAction::None,
        };

        let Some(form) = self.forms.get_mut(fi) else {
            return FormAction::None;
        };

        let Some(elem_idx) = form.index_of(&name) else {
            return FormAction::None;
        };

        // Determine element kind first (immutable peek) so we can
        // dispatch without holding a mutable borrow across the
        // whole match.
        let kind = ElementKind::of(&form.elements[elem_idx]);

        match kind {
            ElementKind::TextInput { maxlength } => {
                Self::handle_text_on_form(&key, form, elem_idx, maxlength)
            },
            ElementKind::TextArea => Self::handle_text_on_form(&key, form, elem_idx, None),
            ElementKind::Checkbox => match key {
                FormKey::Space | FormKey::Enter => {
                    if let FormElement::Checkbox { checked, .. } = &mut form.elements[elem_idx] {
                        *checked = !*checked;
                    }
                    FormAction::ValueChanged
                },
                _ => FormAction::None,
            },
            ElementKind::RadioButton { group, value } => match key {
                FormKey::Space | FormKey::Enter => {
                    for e in &mut form.elements {
                        if let FormElement::RadioButton {
                            group: g,
                            value: v,
                            checked,
                            ..
                        } = e
                            && *g == group
                        {
                            *checked = *v == value;
                        }
                    }
                    FormAction::ValueChanged
                },
                _ => FormAction::None,
            },
            ElementKind::SelectBox => {
                if let FormElement::SelectBox {
                    options,
                    selected_index,
                    ..
                } = &mut form.elements[elem_idx]
                {
                    Self::handle_select_key(&key, options, selected_index)
                } else {
                    FormAction::None
                }
            },
            ElementKind::SubmitButton => match key {
                FormKey::Space | FormKey::Enter => {
                    let data = FormData {
                        fields: form.collect(),
                        method: form.method,
                        action: form.action.clone(),
                    };
                    FormAction::Submit(data)
                },
                _ => FormAction::None,
            },
            ElementKind::ResetButton => match key {
                FormKey::Space | FormKey::Enter => {
                    let defaults = form.defaults.clone();
                    form.elements = defaults;
                    form.cursor = 0;
                    FormAction::ValueChanged
                },
                _ => FormAction::None,
            },
            ElementKind::Hidden => FormAction::None,
        }
    }

    /// Handle a text key for a text input / textarea at `elem_idx`
    /// in `form`, collecting all elements for Enter-submit without
    /// conflicting borrows.
    fn handle_text_on_form(
        key: &FormKey,
        form: &mut FormState,
        elem_idx: usize,
        maxlength: Option<usize>,
    ) -> FormAction {
        // For Enter we need to collect *all* elements, so handle it
        // before taking a mutable borrow on the individual element.
        if *key == FormKey::Enter {
            let fields = collect_from_elements(&form.elements);
            return FormAction::Submit(FormData {
                fields,
                method: form.method,
                action: form.action.clone(),
            });
        }

        let value = match &mut form.elements[elem_idx] {
            FormElement::TextInput { value, .. } | FormElement::TextArea { value, .. } => value,
            _ => return FormAction::None,
        };

        Self::handle_text_key(key, value, &mut form.cursor, maxlength)
    }

    /// Handle a single text-editing key event.
    fn handle_text_key(
        key: &FormKey,
        value: &mut String,
        cursor: &mut usize,
        maxlength: Option<usize>,
    ) -> FormAction {
        match key {
            FormKey::Char(ch) => {
                if let Some(max) = maxlength
                    && value.chars().count() >= max
                {
                    return FormAction::None;
                }
                let byte_pos = char_to_byte(value, *cursor);
                value.insert(byte_pos, *ch);
                *cursor += 1;
                FormAction::ValueChanged
            },
            FormKey::Backspace => {
                if *cursor > 0 {
                    *cursor -= 1;
                    let byte_pos = char_to_byte(value, *cursor);
                    value.remove(byte_pos);
                    FormAction::ValueChanged
                } else {
                    FormAction::None
                }
            },
            FormKey::Delete => {
                let len = value.chars().count();
                if *cursor < len {
                    let byte_pos = char_to_byte(value, *cursor);
                    value.remove(byte_pos);
                    FormAction::ValueChanged
                } else {
                    FormAction::None
                }
            },
            FormKey::Left => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
                FormAction::None
            },
            FormKey::Right => {
                let len = value.chars().count();
                if *cursor < len {
                    *cursor += 1;
                }
                FormAction::None
            },
            FormKey::Home => {
                *cursor = 0;
                FormAction::None
            },
            FormKey::End => {
                *cursor = value.chars().count();
                FormAction::None
            },
            FormKey::Space => {
                if let Some(max) = maxlength
                    && value.chars().count() >= max
                {
                    return FormAction::None;
                }
                let byte_pos = char_to_byte(value, *cursor);
                value.insert(byte_pos, ' ');
                *cursor += 1;
                FormAction::ValueChanged
            },
            // Enter is handled in handle_text_on_form.
            _ => FormAction::None,
        }
    }

    /// Handle a key event on a select box.
    fn handle_select_key(
        key: &FormKey,
        options: &[SelectOption],
        selected_index: &mut Option<usize>,
    ) -> FormAction {
        if options.is_empty() {
            return FormAction::None;
        }

        match key {
            FormKey::Up => {
                let current = selected_index.unwrap_or(0);
                // Search backward for a non-disabled option.
                let mut idx = current;
                loop {
                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                    if !options[idx].disabled {
                        *selected_index = Some(idx);
                        return FormAction::ValueChanged;
                    }
                }
                FormAction::None
            },
            FormKey::Down => {
                let current = selected_index.unwrap_or(0);
                let mut idx = current;
                loop {
                    idx += 1;
                    if idx >= options.len() {
                        break;
                    }
                    if !options[idx].disabled {
                        *selected_index = Some(idx);
                        return FormAction::ValueChanged;
                    }
                }
                FormAction::None
            },
            _ => FormAction::None,
        }
    }

    /// Submit a form by id, returning the collected data.
    pub fn submit(&self, form_id: usize) -> Option<FormData> {
        let form = self.forms.get(form_id)?;
        Some(FormData {
            fields: form.collect(),
            method: form.method,
            action: form.action.clone(),
        })
    }

    /// Reset a form to its default values.
    pub fn reset(&mut self, form_id: usize) {
        if let Some(form) = self.forms.get_mut(form_id) {
            form.reset();
        }
    }

    /// Remove all forms (e.g. on page navigation).
    pub fn clear(&mut self) {
        self.forms.clear();
        self.focused_form = None;
        self.focused_element = None;
    }
}

impl Default for FormManager {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Convert a char-index cursor position to a byte offset.
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Collect form data from a slice of elements (used internally when
/// we only have a shared reference to the elements vec).
fn collect_from_elements(elements: &[FormElement]) -> Vec<(String, String)> {
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

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::types::{InputType, SelectOption};
    use super::*;

    // -- helpers --------------------------------------------------------

    fn make_manager_with_login_form() -> FormManager {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/login", FormMethod::Post);
        mgr.add_element(
            fid,
            FormElement::TextInput {
                name: "user".into(),
                value: String::new(),
                placeholder: "Username".into(),
                maxlength: Some(20),
                input_type: InputType::Text,
            },
        );
        mgr.add_element(
            fid,
            FormElement::TextInput {
                name: "pass".into(),
                value: String::new(),
                placeholder: "Password".into(),
                maxlength: None,
                input_type: InputType::Password,
            },
        );
        mgr.add_element(
            fid,
            FormElement::Checkbox {
                name: "remember".into(),
                value: "1".into(),
                checked: false,
                label: "Remember me".into(),
            },
        );
        mgr.add_element(
            fid,
            FormElement::SubmitButton {
                name: String::new(),
                value: "login".into(),
                label: "Log In".into(),
            },
        );
        mgr
    }

    // -- FormManager creation and element addition ----------------------

    #[test]
    fn new_manager_is_empty() {
        let mgr = FormManager::new();
        assert!(mgr.forms.is_empty());
        assert!(mgr.focused_form.is_none());
        assert!(mgr.focused_element.is_none());
    }

    #[test]
    fn default_manager_is_empty() {
        let mgr = FormManager::default();
        assert!(mgr.forms.is_empty());
    }

    #[test]
    fn add_form_returns_sequential_ids() {
        let mut mgr = FormManager::new();
        let a = mgr.add_form("/a", FormMethod::Get);
        let b = mgr.add_form("/b", FormMethod::Post);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(mgr.forms.len(), 2);
    }

    #[test]
    fn add_element_to_form() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/search", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::TextInput {
                name: "q".into(),
                value: String::new(),
                placeholder: "Search".into(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );
        assert_eq!(mgr.forms[fid].elements.len(), 1);
    }

    #[test]
    fn add_element_invalid_form_id_is_noop() {
        let mut mgr = FormManager::new();
        mgr.add_element(
            99,
            FormElement::TextInput {
                name: "q".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );
        // No panic, no forms created.
        assert!(mgr.forms.is_empty());
    }

    // -- Text input: set/get value, cursor, typing ----------------------

    #[test]
    fn set_and_get_value() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "alice");
        assert_eq!(mgr.get_value(0, "user"), Some("alice"));
    }

    #[test]
    fn get_value_nonexistent_returns_none() {
        let mgr = make_manager_with_login_form();
        assert_eq!(mgr.get_value(0, "nonexistent"), None);
        assert_eq!(mgr.get_value(99, "user"), None);
    }

    #[test]
    fn set_value_respects_maxlength() {
        let mut mgr = make_manager_with_login_form();
        let long = "a".repeat(50);
        mgr.set_value(0, "user", &long);
        // maxlength = 20 for user field.
        assert_eq!(mgr.get_value(0, "user").map(|s| s.len()), Some(20));
    }

    #[test]
    fn type_characters_into_focused_field() {
        let mut mgr = make_manager_with_login_form();
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 0;

        let result = mgr.handle_input(FormKey::Char('h'));
        assert_eq!(result, FormAction::ValueChanged);
        mgr.handle_input(FormKey::Char('i'));
        assert_eq!(mgr.get_value(0, "user"), Some("hi"));
    }

    #[test]
    fn backspace_deletes_before_cursor() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "hello");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 5;

        mgr.handle_input(FormKey::Backspace);
        assert_eq!(mgr.get_value(0, "user"), Some("hell"));
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "hi");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 0;

        let result = mgr.handle_input(FormKey::Backspace);
        assert_eq!(result, FormAction::None);
        assert_eq!(mgr.get_value(0, "user"), Some("hi"));
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "abc");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 1;

        mgr.handle_input(FormKey::Delete);
        assert_eq!(mgr.get_value(0, "user"), Some("ac"));
    }

    #[test]
    fn cursor_movement_left_right() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "abc");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 2;

        mgr.handle_input(FormKey::Left);
        assert_eq!(mgr.forms[0].cursor, 1);

        mgr.handle_input(FormKey::Right);
        assert_eq!(mgr.forms[0].cursor, 2);
    }

    #[test]
    fn cursor_home_and_end() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "hello");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 3;

        mgr.handle_input(FormKey::Home);
        assert_eq!(mgr.forms[0].cursor, 0);

        mgr.handle_input(FormKey::End);
        assert_eq!(mgr.forms[0].cursor, 5);
    }

    #[test]
    fn maxlength_prevents_typing() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::TextInput {
                name: "code".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: Some(3),
                input_type: InputType::Text,
            },
        );
        mgr.focused_form = Some(fid);
        mgr.focused_element = Some("code".into());
        mgr.forms[fid].cursor = 0;

        mgr.handle_input(FormKey::Char('a'));
        mgr.handle_input(FormKey::Char('b'));
        mgr.handle_input(FormKey::Char('c'));
        let result = mgr.handle_input(FormKey::Char('d'));
        assert_eq!(result, FormAction::None);
        assert_eq!(mgr.get_value(fid, "code"), Some("abc"));
    }

    #[test]
    fn space_inserts_space_in_text_field() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "ab");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 1;

        mgr.handle_input(FormKey::Space);
        assert_eq!(mgr.get_value(0, "user"), Some("a b"));
    }

    // -- Checkbox toggle ------------------------------------------------

    #[test]
    fn toggle_checkbox() {
        let mut mgr = make_manager_with_login_form();
        mgr.toggle_checkbox(0, "remember");
        // Check the internal state directly.
        let __out = &mgr.forms[0].elements[2];
        assert!(
            matches!(&__out, FormElement::Checkbox { .. }),
            "expected FormElement::Checkbox, got {__out:?}"
        );
        let FormElement::Checkbox { checked, .. } = __out else {
            unreachable!()
        };
        assert!(*checked);

        mgr.toggle_checkbox(0, "remember");
        let __out = &mgr.forms[0].elements[2];
        assert!(
            matches!(&__out, FormElement::Checkbox { .. }),
            "expected FormElement::Checkbox, got {__out:?}"
        );
        let FormElement::Checkbox { checked, .. } = __out else {
            unreachable!()
        };
        assert!(!*checked);
    }

    #[test]
    fn toggle_checkbox_via_input() {
        let mut mgr = make_manager_with_login_form();
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("remember".into());

        let result = mgr.handle_input(FormKey::Space);
        assert_eq!(result, FormAction::ValueChanged);
    }

    // -- Radio button group selection -----------------------------------

    #[test]
    fn radio_group_single_selection() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::RadioButton {
                name: "color".into(),
                value: "red".into(),
                checked: true,
                group: "color".into(),
            },
        );
        mgr.add_element(
            fid,
            FormElement::RadioButton {
                name: "color".into(),
                value: "blue".into(),
                checked: false,
                group: "color".into(),
            },
        );
        mgr.add_element(
            fid,
            FormElement::RadioButton {
                name: "color".into(),
                value: "green".into(),
                checked: false,
                group: "color".into(),
            },
        );

        mgr.select_radio(fid, "color", "blue");

        let checks: Vec<bool> = mgr.forms[fid]
            .elements
            .iter()
            .filter_map(|e| match e {
                FormElement::RadioButton { checked, .. } => Some(*checked),
                _ => None,
            })
            .collect();
        assert_eq!(checks, vec![false, true, false]);
    }

    #[test]
    fn radio_toggle_via_input() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        // Use distinct names so focus lookup works: the first radio
        // with name "color" is at index 0.  We focus it by name.
        mgr.add_element(
            fid,
            FormElement::RadioButton {
                name: "color".into(),
                value: "red".into(),
                checked: false,
                group: "color".into(),
            },
        );
        mgr.focused_form = Some(fid);
        mgr.focused_element = Some("color".into());

        let result = mgr.handle_input(FormKey::Enter);
        assert_eq!(result, FormAction::ValueChanged);
    }

    // -- Select dropdown ------------------------------------------------

    #[test]
    fn select_option_by_index() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "size".into(),
                options: vec![
                    SelectOption {
                        value: "s".into(),
                        label: "Small".into(),
                        disabled: false,
                    },
                    SelectOption {
                        value: "m".into(),
                        label: "Medium".into(),
                        disabled: false,
                    },
                    SelectOption {
                        value: "l".into(),
                        label: "Large".into(),
                        disabled: false,
                    },
                ],
                selected_index: None,
            },
        );

        mgr.select_option(fid, "size", 1);
        let __out = &mgr.forms[fid].elements[0];
        assert!(
            matches!(&__out, FormElement::SelectBox { .. }),
            "expected FormElement::SelectBox, got {__out:?}"
        );
        let FormElement::SelectBox { selected_index, .. } = __out else {
            unreachable!()
        };
        assert_eq!(*selected_index, Some(1));
    }

    #[test]
    fn select_disabled_option_is_ignored() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "size".into(),
                options: vec![
                    SelectOption {
                        value: "s".into(),
                        label: "Small".into(),
                        disabled: false,
                    },
                    SelectOption {
                        value: "m".into(),
                        label: "Medium".into(),
                        disabled: true,
                    },
                ],
                selected_index: Some(0),
            },
        );

        mgr.select_option(fid, "size", 1);
        // Should remain at 0 because index 1 is disabled.
        let __out = &mgr.forms[fid].elements[0];
        assert!(
            matches!(&__out, FormElement::SelectBox { .. }),
            "expected FormElement::SelectBox, got {__out:?}"
        );
        let FormElement::SelectBox { selected_index, .. } = __out else {
            unreachable!()
        };
        assert_eq!(*selected_index, Some(0));
    }

    #[test]
    fn select_navigate_with_arrow_keys() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "fruit".into(),
                options: vec![
                    SelectOption {
                        value: "a".into(),
                        label: "Apple".into(),
                        disabled: false,
                    },
                    SelectOption {
                        value: "b".into(),
                        label: "Banana".into(),
                        disabled: false,
                    },
                    SelectOption {
                        value: "c".into(),
                        label: "Cherry".into(),
                        disabled: false,
                    },
                ],
                selected_index: Some(0),
            },
        );
        mgr.focused_form = Some(fid);
        mgr.focused_element = Some("fruit".into());

        let r = mgr.handle_input(FormKey::Down);
        assert_eq!(r, FormAction::ValueChanged);
        if let FormElement::SelectBox { selected_index, .. } = &mgr.forms[fid].elements[0] {
            assert_eq!(*selected_index, Some(1));
        }

        mgr.handle_input(FormKey::Up);
        if let FormElement::SelectBox { selected_index, .. } = &mgr.forms[fid].elements[0] {
            assert_eq!(*selected_index, Some(0));
        }
    }

    #[test]
    fn select_arrow_skips_disabled() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "x".into(),
                options: vec![
                    SelectOption {
                        value: "0".into(),
                        label: "Zero".into(),
                        disabled: false,
                    },
                    SelectOption {
                        value: "1".into(),
                        label: "One".into(),
                        disabled: true,
                    },
                    SelectOption {
                        value: "2".into(),
                        label: "Two".into(),
                        disabled: false,
                    },
                ],
                selected_index: Some(0),
            },
        );
        mgr.focused_form = Some(fid);
        mgr.focused_element = Some("x".into());

        mgr.handle_input(FormKey::Down);
        if let FormElement::SelectBox { selected_index, .. } = &mgr.forms[fid].elements[0] {
            // Should skip index 1 (disabled) and land on 2.
            assert_eq!(*selected_index, Some(2));
        }
    }

    // -- Tab navigation -------------------------------------------------

    #[test]
    fn tab_cycles_through_focusable_elements() {
        let mut mgr = make_manager_with_login_form();

        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_element.as_deref(), Some("user"));

        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_element.as_deref(), Some("pass"));

        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_element.as_deref(), Some("remember"));

        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_element.as_deref(), Some("__submit__"));

        // Wraps around.
        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_element.as_deref(), Some("user"));
    }

    #[test]
    fn shift_tab_goes_backward() {
        let mut mgr = make_manager_with_login_form();

        // First tab focuses "user".
        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_element.as_deref(), Some("user"));

        // Shift-tab wraps to last element.
        mgr.handle_input(FormKey::ShiftTab);
        assert_eq!(mgr.focused_element.as_deref(), Some("__submit__"));
    }

    #[test]
    fn tab_skips_hidden_inputs() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::HiddenInput {
                name: "token".into(),
                value: "abc".into(),
            },
        );
        mgr.add_element(
            fid,
            FormElement::TextInput {
                name: "q".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );

        mgr.handle_input(FormKey::Tab);
        // Should skip hidden and focus "q".
        assert_eq!(mgr.focused_element.as_deref(), Some("q"));
    }

    // -- Form submission ------------------------------------------------

    #[test]
    fn submit_collects_all_values() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "alice");
        mgr.set_value(0, "pass", "secret");
        mgr.toggle_checkbox(0, "remember");

        let data = mgr.submit(0).expect("submit should succeed");
        assert_eq!(data.action, "/login");
        assert_eq!(data.method, FormMethod::Post);

        let pairs = data.to_pairs();
        assert!(pairs.contains(&("user".to_string(), "alice".to_string())));
        assert!(pairs.contains(&("pass".to_string(), "secret".to_string())));
        assert!(pairs.contains(&("remember".to_string(), "1".to_string())));
    }

    #[test]
    fn unchecked_checkbox_not_in_submission() {
        let mgr = make_manager_with_login_form();
        let data = mgr.submit(0).expect("submit should succeed");
        let pairs = data.to_pairs();
        assert!(!pairs.iter().any(|(k, _)| k == "remember"));
    }

    #[test]
    fn submit_via_enter_in_text_field() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "bob");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 3;

        let result = mgr.handle_input(FormKey::Enter);
        let __out = result;
        assert!(
            matches!(&__out, FormAction::Submit(_)),
            "expected FormAction::Submit, got {__out:?}"
        );
        let FormAction::Submit(data) = __out else {
            unreachable!()
        };
        assert_eq!(data.action, "/login");
    }

    #[test]
    fn submit_nonexistent_form_returns_none() {
        let mgr = FormManager::new();
        assert!(mgr.submit(0).is_none());
    }

    // -- Form reset -----------------------------------------------------

    #[test]
    fn reset_restores_defaults() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "alice");
        mgr.toggle_checkbox(0, "remember");

        mgr.reset(0);

        assert_eq!(mgr.get_value(0, "user"), Some(""));
        if let FormElement::Checkbox { checked, .. } = &mgr.forms[0].elements[2] {
            assert!(!*checked);
        }
    }

    #[test]
    fn reset_via_button() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::TextInput {
                name: "q".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );
        mgr.add_element(
            fid,
            FormElement::ResetButton {
                label: "Reset".into(),
            },
        );

        mgr.set_value(fid, "q", "something");
        mgr.focused_form = Some(fid);
        mgr.focused_element = Some("__reset__".into());

        let result = mgr.handle_input(FormKey::Enter);
        assert_eq!(result, FormAction::ValueChanged);
        assert_eq!(mgr.get_value(fid, "q"), Some(""));
    }

    // -- URL encoding ---------------------------------------------------

    #[test]
    fn url_encode_simple() {
        let data = FormData {
            fields: vec![
                ("q".into(), "hello world".into()),
                ("lang".into(), "en".into()),
            ],
            method: FormMethod::Get,
            action: "/search".into(),
        };
        assert_eq!(data.encode(), "q=hello+world&lang=en");
    }

    #[test]
    fn url_encode_special_characters() {
        let data = FormData {
            fields: vec![("data".into(), "a=1&b=2".into())],
            method: FormMethod::Get,
            action: "/".into(),
        };
        assert_eq!(data.encode(), "data=a%3D1%26b%3D2");
    }

    #[test]
    fn url_encode_empty_form() {
        let data = FormData {
            fields: vec![],
            method: FormMethod::Get,
            action: "/".into(),
        };
        assert_eq!(data.encode(), "");
    }

    // -- Multiple forms on same page ------------------------------------

    #[test]
    fn multiple_forms_independent_state() {
        let mut mgr = FormManager::new();
        let f0 = mgr.add_form("/search", FormMethod::Get);
        let f1 = mgr.add_form("/login", FormMethod::Post);

        mgr.add_element(
            f0,
            FormElement::TextInput {
                name: "q".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );
        mgr.add_element(
            f1,
            FormElement::TextInput {
                name: "q".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );

        mgr.set_value(f0, "q", "search term");
        mgr.set_value(f1, "q", "login user");

        assert_eq!(mgr.get_value(f0, "q"), Some("search term"));
        assert_eq!(mgr.get_value(f1, "q"), Some("login user"));
    }

    #[test]
    fn tab_across_multiple_forms() {
        let mut mgr = FormManager::new();
        let f0 = mgr.add_form("/a", FormMethod::Get);
        let f1 = mgr.add_form("/b", FormMethod::Get);

        mgr.add_element(
            f0,
            FormElement::TextInput {
                name: "x".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );
        mgr.add_element(
            f1,
            FormElement::TextInput {
                name: "y".into(),
                value: String::new(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );

        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_form, Some(0));
        assert_eq!(mgr.focused_element.as_deref(), Some("x"));

        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_form, Some(1));
        assert_eq!(mgr.focused_element.as_deref(), Some("y"));

        mgr.handle_input(FormKey::Tab);
        assert_eq!(mgr.focused_form, Some(0));
        assert_eq!(mgr.focused_element.as_deref(), Some("x"));
    }

    // -- Edge cases -----------------------------------------------------

    #[test]
    fn empty_form_submit_returns_empty_fields() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/empty", FormMethod::Get);
        let data = mgr.submit(fid).expect("should work");
        assert!(data.fields.is_empty());
    }

    #[test]
    fn hidden_input_included_in_submission() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Post);
        mgr.add_element(
            fid,
            FormElement::HiddenInput {
                name: "csrf".into(),
                value: "tok123".into(),
            },
        );

        let data = mgr.submit(fid).expect("should work");
        assert_eq!(data.fields, vec![("csrf".into(), "tok123".into())]);
    }

    #[test]
    fn hidden_input_value_readable() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::HiddenInput {
                name: "tok".into(),
                value: "abc".into(),
            },
        );
        assert_eq!(mgr.get_value(fid, "tok"), Some("abc"));
    }

    #[test]
    fn clear_removes_everything() {
        let mut mgr = make_manager_with_login_form();
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());

        mgr.clear();

        assert!(mgr.forms.is_empty());
        assert!(mgr.focused_form.is_none());
        assert!(mgr.focused_element.is_none());
    }

    #[test]
    fn input_with_no_focus_returns_none() {
        let mut mgr = make_manager_with_login_form();
        let result = mgr.handle_input(FormKey::Char('a'));
        assert_eq!(result, FormAction::None);
    }

    #[test]
    fn select_with_no_options_arrow_is_noop() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "empty".into(),
                options: vec![],
                selected_index: None,
            },
        );
        mgr.focused_form = Some(fid);
        mgr.focused_element = Some("empty".into());

        let result = mgr.handle_input(FormKey::Down);
        assert_eq!(result, FormAction::None);
    }

    #[test]
    fn textarea_set_get_value() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Post);
        mgr.add_element(
            fid,
            FormElement::TextArea {
                name: "body".into(),
                value: String::new(),
                rows: 5,
                cols: 40,
                placeholder: "Write here".into(),
            },
        );

        mgr.set_value(fid, "body", "Hello\nWorld");
        assert_eq!(mgr.get_value(fid, "body"), Some("Hello\nWorld"));
    }

    #[test]
    fn textarea_included_in_submission() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/post", FormMethod::Post);
        mgr.add_element(
            fid,
            FormElement::TextArea {
                name: "content".into(),
                value: "some text".into(),
                rows: 3,
                cols: 40,
                placeholder: String::new(),
            },
        );

        let data = mgr.submit(fid).expect("should work");
        assert_eq!(data.fields, vec![("content".into(), "some text".into())]);
    }

    #[test]
    fn select_box_value_in_submission() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/order", FormMethod::Post);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "size".into(),
                options: vec![
                    SelectOption {
                        value: "s".into(),
                        label: "Small".into(),
                        disabled: false,
                    },
                    SelectOption {
                        value: "m".into(),
                        label: "Medium".into(),
                        disabled: false,
                    },
                ],
                selected_index: Some(1),
            },
        );

        let data = mgr.submit(fid).expect("should work");
        assert_eq!(data.fields, vec![("size".into(), "m".into())]);
    }

    #[test]
    fn select_no_selection_omitted() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "x".into(),
                options: vec![SelectOption {
                    value: "a".into(),
                    label: "A".into(),
                    disabled: false,
                }],
                selected_index: None,
            },
        );

        let data = mgr.submit(fid).expect("should work");
        assert!(data.fields.is_empty());
    }

    #[test]
    fn out_of_range_select_option_ignored() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::SelectBox {
                name: "x".into(),
                options: vec![SelectOption {
                    value: "a".into(),
                    label: "A".into(),
                    disabled: false,
                }],
                selected_index: Some(0),
            },
        );

        mgr.select_option(fid, "x", 99);
        if let FormElement::SelectBox { selected_index, .. } = &mgr.forms[fid].elements[0] {
            assert_eq!(*selected_index, Some(0));
        }
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "hi");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 2;

        let result = mgr.handle_input(FormKey::Delete);
        assert_eq!(result, FormAction::None);
    }

    #[test]
    fn cursor_left_clamped_at_zero() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "abc");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 0;

        mgr.handle_input(FormKey::Left);
        assert_eq!(mgr.forms[0].cursor, 0);
    }

    #[test]
    fn cursor_right_clamped_at_len() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "ab");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 2;

        mgr.handle_input(FormKey::Right);
        assert_eq!(mgr.forms[0].cursor, 2);
    }

    #[test]
    fn insert_at_middle_of_text() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "ac");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("user".into());
        mgr.forms[0].cursor = 1;

        mgr.handle_input(FormKey::Char('b'));
        assert_eq!(mgr.get_value(0, "user"), Some("abc"));
        assert_eq!(mgr.forms[0].cursor, 2);
    }

    #[test]
    fn submit_button_via_input() {
        let mut mgr = make_manager_with_login_form();
        mgr.set_value(0, "user", "test");
        mgr.focused_form = Some(0);
        mgr.focused_element = Some("__submit__".into());

        let result = mgr.handle_input(FormKey::Enter);
        let __out = result;
        assert!(
            matches!(&__out, FormAction::Submit(_)),
            "expected FormAction::Submit, got {__out:?}"
        );
        let FormAction::Submit(data) = __out else {
            unreachable!()
        };
        assert_eq!(data.action, "/login");
        assert!(data.fields.iter().any(|(k, v)| k == "user" && v == "test"));
    }

    #[test]
    fn form_data_to_pairs() {
        let data = FormData {
            fields: vec![("a".into(), "1".into()), ("b".into(), "2".into())],
            method: FormMethod::Get,
            action: "/".into(),
        };
        let pairs = data.to_pairs();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("a".into(), "1".into()));
    }

    #[test]
    fn url_encode_preserves_unreserved() {
        use super::super::types::url_encode;
        assert_eq!(url_encode("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn url_encode_percent_encodes_reserved() {
        use super::super::types::url_encode;
        assert_eq!(url_encode("@"), "%40");
        assert_eq!(url_encode("/"), "%2F");
    }

    #[test]
    fn focus_next_sets_cursor_to_end() {
        let mut mgr = FormManager::new();
        let fid = mgr.add_form("/", FormMethod::Get);
        mgr.add_element(
            fid,
            FormElement::TextInput {
                name: "a".into(),
                value: "hello".into(),
                placeholder: String::new(),
                maxlength: None,
                input_type: InputType::Text,
            },
        );

        mgr.focus_next();
        assert_eq!(mgr.forms[fid].cursor, 5);
    }
}
