//! Keyboard input handling and focus navigation for forms.

use super::manager::FormManager;
use super::serialization::collect_from_elements;
use super::state::{ElementKind, FormState};
use super::types::{FormAction, FormData, FormElement, FormKey, SelectOption};

impl FormManager {
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
                    form.cursor = value.chars().count();
                },
                FormElement::TextArea { value, .. } => {
                    form.cursor = value.chars().count();
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
                    open,
                    ..
                } = &mut form.elements[elem_idx]
                {
                    Self::handle_select_key(&key, options, selected_index, open)
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
                let byte_pos = super::serialization::char_to_byte(value, *cursor);
                value.insert(byte_pos, *ch);
                *cursor += 1;
                FormAction::ValueChanged
            },
            FormKey::Backspace => {
                if *cursor > 0 {
                    *cursor -= 1;
                    let byte_pos = super::serialization::char_to_byte(value, *cursor);
                    value.remove(byte_pos);
                    FormAction::ValueChanged
                } else {
                    FormAction::None
                }
            },
            FormKey::Delete => {
                let len = value.chars().count();
                if *cursor < len {
                    let byte_pos = super::serialization::char_to_byte(value, *cursor);
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
                let byte_pos = super::serialization::char_to_byte(value, *cursor);
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
        open: &mut bool,
    ) -> FormAction {
        if options.is_empty() {
            return FormAction::None;
        }

        match key {
            FormKey::Space | FormKey::Enter => {
                *open = !*open;
                FormAction::ValueChanged
            },
            FormKey::Char('\u{001B}') => {
                if *open {
                    *open = false;
                    FormAction::ValueChanged
                } else {
                    FormAction::None
                }
            },
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
}
