//! Form validation: check constraints before submission.
//!
//! Validates `required`, `minlength`, `maxlength`, `pattern`, `min`,
//! and `max` attributes on form elements.

use super::types::{FormElement, InputType};

/// A single validation error for one form element.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    /// Index of the element within the form's element list.
    pub element_index: usize,
    /// Human-readable error message.
    pub message: String,
}

/// Validate all elements in a form, returning any constraint
/// violations.
///
/// An empty `Vec` means the form is valid.
pub fn validate_form(elements: &[FormElement]) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for (idx, elem) in elements.iter().enumerate() {
        match elem {
            FormElement::TextInput {
                value,
                input_type,
                required,
                minlength,
                maxlength,
                pattern,
                min,
                max,
                ..
            } => {
                let trimmed = value.trim();

                // Required check.
                if *required && trimmed.is_empty() {
                    errors.push(ValidationError {
                        element_index: idx,
                        message: "This field is required".to_string(),
                    });
                    continue;
                }

                // Skip remaining checks for empty optional fields.
                if trimmed.is_empty() {
                    continue;
                }

                // Minlength check.
                if let Some(min_len) = minlength
                    && value.chars().count() < *min_len
                {
                    errors.push(ValidationError {
                        element_index: idx,
                        message: format!("Must be at least {} characters", min_len),
                    });
                }

                // Maxlength check.
                if let Some(max_len) = maxlength
                    && value.chars().count() > *max_len
                {
                    errors.push(ValidationError {
                        element_index: idx,
                        message: format!("Must be no more than {} characters", max_len),
                    });
                }

                // Pattern check (simple substring/prefix match — no
                // regex crate dependency).
                if let Some(pat) = pattern
                    && !simple_pattern_match(pat, value)
                {
                    errors.push(ValidationError {
                        element_index: idx,
                        message: format!("Value does not match pattern \"{}\"", pat),
                    });
                }

                // Min / max for number inputs.
                if *input_type == InputType::Number
                    && let Ok(num) = value.parse::<f64>()
                {
                    if let Some(lo) = min
                        && num < *lo
                    {
                        errors.push(ValidationError {
                            element_index: idx,
                            message: format!("Value must be at least {}", lo),
                        });
                    }
                    if let Some(hi) = max
                        && num > *hi
                    {
                        errors.push(ValidationError {
                            element_index: idx,
                            message: format!("Value must be no more than {}", hi),
                        });
                    }
                }
            },
            FormElement::TextArea {
                value,
                required,
                minlength,
                maxlength,
                ..
            } => {
                let trimmed = value.trim();

                if *required && trimmed.is_empty() {
                    errors.push(ValidationError {
                        element_index: idx,
                        message: "This field is required".to_string(),
                    });
                    continue;
                }

                if trimmed.is_empty() {
                    continue;
                }

                if let Some(min_len) = minlength
                    && value.chars().count() < *min_len
                {
                    errors.push(ValidationError {
                        element_index: idx,
                        message: format!("Must be at least {} characters", min_len),
                    });
                }

                if let Some(max_len) = maxlength
                    && value.chars().count() > *max_len
                {
                    errors.push(ValidationError {
                        element_index: idx,
                        message: format!("Must be no more than {} characters", max_len),
                    });
                }
            },
            // Other element types have no validation constraints.
            _ => {},
        }
    }

    errors
}

/// Simple pattern matching without a regex engine.
///
/// Supports three forms:
/// - `^...$` — exact match (anchored both ends)
/// - `^...` — prefix match
/// - `...$` — suffix match
/// - anything else — substring containment
fn simple_pattern_match(pattern: &str, value: &str) -> bool {
    let starts = pattern.starts_with('^');
    let ends = pattern.ends_with('$');

    match (starts, ends) {
        (true, true) => {
            // Exact match: strip anchors.
            let inner = &pattern[1..pattern.len() - 1];
            value == inner
        },
        (true, false) => {
            let inner = &pattern[1..];
            value.starts_with(inner)
        },
        (false, true) => {
            let inner = &pattern[..pattern.len() - 1];
            value.ends_with(inner)
        },
        (false, false) => value.contains(pattern),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a basic text input with defaults.
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

    fn required_text(name: &str, value: &str) -> FormElement {
        FormElement::TextInput {
            name: name.into(),
            value: value.into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: true,
            minlength: None,
            pattern: None,
            min: None,
            max: None,
        }
    }

    fn number_input(value: &str, min: Option<f64>, max: Option<f64>) -> FormElement {
        FormElement::TextInput {
            name: "num".into(),
            value: value.into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Number,
            required: false,
            minlength: None,
            pattern: None,
            min,
            max,
        }
    }

    // -- required -------------------------------------------------

    #[test]
    fn required_empty_fails() {
        let elems = vec![required_text("user", "")];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].element_index, 0);
        assert_eq!(errs[0].message, "This field is required");
    }

    #[test]
    fn required_whitespace_only_fails() {
        let elems = vec![required_text("user", "   ")];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].message, "This field is required");
    }

    #[test]
    fn required_with_value_passes() {
        let elems = vec![required_text("user", "alice")];
        assert!(validate_form(&elems).is_empty());
    }

    #[test]
    fn optional_empty_passes() {
        let elems = vec![text_input("user", "")];
        assert!(validate_form(&elems).is_empty());
    }

    // -- minlength ------------------------------------------------

    #[test]
    fn minlength_violated() {
        let elems = vec![FormElement::TextInput {
            name: "pw".into(),
            value: "ab".into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: Some(5),
            pattern: None,
            min: None,
            max: None,
        }];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("at least 5"));
    }

    #[test]
    fn minlength_satisfied() {
        let elems = vec![FormElement::TextInput {
            name: "pw".into(),
            value: "abcde".into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: Some(5),
            pattern: None,
            min: None,
            max: None,
        }];
        assert!(validate_form(&elems).is_empty());
    }

    // -- maxlength ------------------------------------------------

    #[test]
    fn maxlength_violated() {
        let elems = vec![FormElement::TextInput {
            name: "code".into(),
            value: "abcdef".into(),
            placeholder: String::new(),
            maxlength: Some(3),
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: None,
            min: None,
            max: None,
        }];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("no more than 3"));
    }

    // -- pattern --------------------------------------------------

    #[test]
    fn pattern_exact_match() {
        let elems = vec![FormElement::TextInput {
            name: "code".into(),
            value: "ABC".into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: Some("^ABC$".into()),
            min: None,
            max: None,
        }];
        assert!(validate_form(&elems).is_empty());
    }

    #[test]
    fn pattern_exact_mismatch() {
        let elems = vec![FormElement::TextInput {
            name: "code".into(),
            value: "ABCD".into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: Some("^ABC$".into()),
            min: None,
            max: None,
        }];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("pattern"));
    }

    #[test]
    fn pattern_prefix() {
        let elems = vec![FormElement::TextInput {
            name: "x".into(),
            value: "hello world".into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: Some("^hello".into()),
            min: None,
            max: None,
        }];
        assert!(validate_form(&elems).is_empty());
    }

    #[test]
    fn pattern_substring() {
        let elems = vec![FormElement::TextInput {
            name: "x".into(),
            value: "the quick fox".into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: Some("quick".into()),
            min: None,
            max: None,
        }];
        assert!(validate_form(&elems).is_empty());
    }

    #[test]
    fn pattern_empty_value_skipped() {
        let elems = vec![FormElement::TextInput {
            name: "x".into(),
            value: String::new(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: Some("^ABC$".into()),
            min: None,
            max: None,
        }];
        // Empty optional field should not trigger pattern check.
        assert!(validate_form(&elems).is_empty());
    }

    // -- min / max for number inputs ------------------------------

    #[test]
    fn number_below_min() {
        let elems = vec![number_input("3", Some(5.0), None)];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("at least 5"));
    }

    #[test]
    fn number_above_max() {
        let elems = vec![number_input("15", None, Some(10.0))];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("no more than 10"));
    }

    #[test]
    fn number_in_range() {
        let elems = vec![number_input("7", Some(1.0), Some(10.0))];
        assert!(validate_form(&elems).is_empty());
    }

    #[test]
    fn number_min_max_not_checked_for_text() {
        // min/max should not apply to non-number inputs.
        let elems = vec![FormElement::TextInput {
            name: "x".into(),
            value: "abc".into(),
            placeholder: String::new(),
            maxlength: None,
            input_type: InputType::Text,
            required: false,
            minlength: None,
            pattern: None,
            min: Some(5.0),
            max: Some(10.0),
        }];
        assert!(validate_form(&elems).is_empty());
    }

    // -- textarea -------------------------------------------------

    #[test]
    fn textarea_required_empty() {
        let elems = vec![FormElement::TextArea {
            name: "body".into(),
            value: String::new(),
            rows: 3,
            cols: 40,
            placeholder: String::new(),
            required: true,
            minlength: None,
            maxlength: None,
        }];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].message, "This field is required");
    }

    #[test]
    fn textarea_minlength() {
        let elems = vec![FormElement::TextArea {
            name: "body".into(),
            value: "hi".into(),
            rows: 3,
            cols: 40,
            placeholder: String::new(),
            required: false,
            minlength: Some(10),
            maxlength: None,
        }];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("at least 10"));
    }

    #[test]
    fn textarea_maxlength() {
        let elems = vec![FormElement::TextArea {
            name: "body".into(),
            value: "a".repeat(100),
            rows: 3,
            cols: 40,
            placeholder: String::new(),
            required: false,
            minlength: None,
            maxlength: Some(50),
        }];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("no more than 50"));
    }

    // -- multiple errors ------------------------------------------

    #[test]
    fn multiple_elements_multiple_errors() {
        let elems = vec![
            required_text("user", ""),
            required_text("email", ""),
            text_input("optional", ""),
        ];
        let errs = validate_form(&elems);
        assert_eq!(errs.len(), 2);
        assert_eq!(errs[0].element_index, 0);
        assert_eq!(errs[1].element_index, 1);
    }

    #[test]
    fn no_errors_when_all_valid() {
        let elems = vec![
            required_text("user", "alice"),
            text_input("bio", "something"),
        ];
        assert!(validate_form(&elems).is_empty());
    }

    // -- non-text elements pass through ---------------------------

    #[test]
    fn checkbox_and_radio_always_valid() {
        let elems = vec![
            FormElement::Checkbox {
                name: "agree".into(),
                value: "yes".into(),
                checked: false,
                label: "I agree".into(),
            },
            FormElement::RadioButton {
                name: "color".into(),
                value: "red".into(),
                checked: false,
                group: "colors".into(),
            },
        ];
        assert!(validate_form(&elems).is_empty());
    }

    // -- simple_pattern_match -------------------------------------

    #[test]
    fn pattern_suffix() {
        assert!(simple_pattern_match("world$", "hello world"));
        assert!(!simple_pattern_match("world$", "world!"));
    }

    #[test]
    fn pattern_substring_no_match() {
        assert!(!simple_pattern_match("xyz", "hello world"));
    }
}
