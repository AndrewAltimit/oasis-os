//! Form keys, DOM <-> form-state sync and submission.

use super::*;

impl BrowserWidget {
    /// Dispatch a form key event to the form manager and handle the
    /// resulting action (submission, focus change, etc.).
    ///
    /// Returns `true` if the event was consumed by the form manager.
    pub fn dispatch_form_key(&mut self, key: crate::forms::FormKey, vfs: &dyn Vfs) -> bool {
        let action = self.form_manager.handle_input(key);
        // Sync form_manager's value back to the DOM so the next
        // relayout paints the typed characters. Without this, the
        // layout tree's `ReplacedContent::TextInput { value }` stays
        // whatever the HTML `<input value="">` attribute said at page
        // load, and the search box appears to ignore every key.
        if matches!(action, crate::forms::FormAction::ValueChanged) {
            self.sync_form_values_to_dom();
        }
        match action {
            crate::forms::FormAction::Submit(ref data) => {
                self.handle_form_submit(data, vfs);
                true
            },
            crate::forms::FormAction::FocusChanged | crate::forms::FormAction::ValueChanged => true,
            crate::forms::FormAction::None => false,
        }
    }

    /// Write every form-manager value back onto the corresponding DOM
    /// element's `value` attribute.
    ///
    /// This is the glue between `form_manager` (source of truth for
    /// in-flight edits) and the layout tree (rebuilt from DOM on every
    /// `layout_dirty` tick). Without this sync the input content is
    /// invisible to the user because the layout pass keeps re-reading
    /// the original HTML attribute.
    ///
    /// The script-side document (`js_doc`) gets the same values, so
    /// `input.value` read by a handler sees what the user typed, and a
    /// later script DOM mutation (which replaces the rendered document
    /// with a copy of the script-side one) doesn't wipe it.
    pub(super) fn sync_form_values_to_dom(&mut self) {
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        Self::write_form_values(doc, &self.form_manager);
        #[cfg(feature = "javascript")]
        if let Some(js_doc) = &self.js_doc {
            Self::write_form_values(&mut js_doc.borrow_mut(), &self.form_manager);
        }
        self.layout_dirty = true;
    }

    /// `<form>` node ids in document order. `populate_forms_from_dom`
    /// registers forms in this same order, so the N-th `<form>` node
    /// corresponds to `form_manager.forms[N]`.
    pub(super) fn form_node_ids(doc: &html::dom::Document) -> Vec<NodeId> {
        use crate::html::dom::{NodeKind, TagName};
        doc.nodes
            .iter()
            .enumerate()
            .filter(
                |(_, node)| matches!(&node.kind, NodeKind::Element(e) if e.tag == TagName::Form),
            )
            .map(|(nid, _)| nid)
            .collect()
    }

    /// Every descendant node id of `form_nid`. Scoping name lookups to
    /// a form's subtree keeps two forms sharing a field name (e.g. both
    /// with `<input name="q">`) from overwriting each other's values.
    pub(super) fn form_descendants(doc: &html::dom::Document, form_nid: NodeId) -> Vec<NodeId> {
        let mut descendants: Vec<usize> = Vec::new();
        let mut stack: Vec<usize> = doc.nodes[form_nid].children.clone();
        while let Some(nid) = stack.pop() {
            descendants.push(nid);
            stack.extend(doc.nodes[nid].children.iter().copied());
        }
        descendants
    }

    /// Write every form-manager value onto `doc`'s form controls.
    pub(super) fn write_form_values(
        doc: &mut html::dom::Document,
        forms: &crate::forms::FormManager,
    ) {
        use crate::forms::FormElement;
        use crate::html::dom::{NodeKind, TagName};
        let form_dom_ids = Self::form_node_ids(doc);
        for (idx, form) in forms.forms.iter().enumerate() {
            let Some(&form_nid) = form_dom_ids.get(idx) else {
                continue;
            };
            let descendants = Self::form_descendants(doc, form_nid);
            for elem in form.elements() {
                match elem {
                    FormElement::TextInput { name, value, .. }
                    | FormElement::TextArea { name, value, .. }
                        if !name.is_empty() =>
                    {
                        for &nid in &descendants {
                            if let NodeKind::Element(ref mut e) = doc.nodes[nid].kind
                                && matches!(e.tag, TagName::Input | TagName::Textarea)
                                && e.get_attribute("name") == Some(name.as_str())
                            {
                                e.set_attribute("value", value);
                            }
                        }
                    },
                    // Checked state lives in the `checked` attribute,
                    // which layout reads to draw the control.
                    FormElement::Checkbox {
                        name,
                        value,
                        checked,
                        ..
                    }
                    | FormElement::RadioButton {
                        name,
                        value,
                        checked,
                        ..
                    } if !name.is_empty() => {
                        for &nid in &descendants {
                            if let NodeKind::Element(ref mut e) = doc.nodes[nid].kind
                                && e.tag == TagName::Input
                                && e.get_attribute("name") == Some(name.as_str())
                                && e.get_attribute("value").unwrap_or("on") == value
                                && matches!(e.get_attribute("type"), Some("checkbox" | "radio"))
                            {
                                if *checked {
                                    e.set_attribute("checked", "");
                                } else {
                                    e.remove_attribute("checked");
                                }
                            }
                        }
                    },
                    // The selected option is the one carrying `selected`.
                    FormElement::SelectBox {
                        name,
                        selected_index,
                        ..
                    } if !name.is_empty() => {
                        let Some(select_nid) = descendants.iter().copied().find(|&nid| {
                            matches!(&doc.nodes[nid].kind, NodeKind::Element(e)
                                if e.tag == TagName::Select
                                    && e.get_attribute("name") == Some(name.as_str()))
                        }) else {
                            continue;
                        };
                        // <option>s in document order (possibly inside
                        // <optgroup>s), matching `populate_forms_from_dom`.
                        let mut options = Vec::new();
                        let mut stack: Vec<usize> = doc.nodes[select_nid]
                            .children
                            .iter()
                            .rev()
                            .copied()
                            .collect();
                        while let Some(nid) = stack.pop() {
                            if let NodeKind::Element(e) = &doc.nodes[nid].kind {
                                if e.tag == TagName::Option {
                                    options.push(nid);
                                    continue;
                                }
                                stack.extend(doc.nodes[nid].children.iter().rev().copied());
                            }
                        }
                        for (i, nid) in options.into_iter().enumerate() {
                            if let NodeKind::Element(ref mut e) = doc.nodes[nid].kind {
                                if Some(i) == *selected_index {
                                    e.set_attribute("selected", "");
                                } else {
                                    e.remove_attribute("selected");
                                }
                            }
                        }
                    },
                    _ => {},
                }
            }
        }
    }

    /// Take text values a script wrote into the DOM (`input.value = ...`
    /// on a text field, textarea or hidden input) into the form manager,
    /// so they are what's submitted and what the next keystroke edits.
    /// Returns `true` if any value changed.
    ///
    /// Typed values are mirrored into the DOM as they change
    /// ([`Self::sync_form_values_to_dom`]), so a DOM value that differs
    /// from the form manager's was set by script.
    #[cfg(feature = "javascript")]
    pub(super) fn adopt_form_values_from_dom(&mut self) -> bool {
        use crate::forms::FormElement;
        use crate::html::dom::{NodeKind, TagName};
        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let form_dom_ids = Self::form_node_ids(doc);
        let mut adopted: Vec<(usize, String, String)> = Vec::new();
        for (idx, form) in self.form_manager.forms.iter().enumerate() {
            let Some(&form_nid) = form_dom_ids.get(idx) else {
                continue;
            };
            let descendants = Self::form_descendants(doc, form_nid);
            for elem in form.elements() {
                let (FormElement::TextInput { name, value, .. }
                | FormElement::TextArea { name, value, .. }
                | FormElement::HiddenInput { name, value }) = elem
                else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let dom_value = descendants
                    .iter()
                    .find_map(|&nid| match &doc.nodes[nid].kind {
                        NodeKind::Element(e)
                            if matches!(e.tag, TagName::Input | TagName::Textarea)
                                && e.get_attribute("name") == Some(name.as_str()) =>
                        {
                            e.get_attribute("value")
                        },
                        _ => None,
                    });
                if let Some(v) = dom_value
                    && v != value
                {
                    adopted.push((idx, name.clone(), v.to_string()));
                }
            }
        }
        let mut changed = false;
        for (idx, name, value) in adopted {
            changed |= self.form_manager.adopt_value(idx, &name, &value);
        }
        changed
    }

    /// Handle a form submission.
    ///
    /// For GET forms, the encoded data is appended as a query string.
    /// For POST forms, the encoded data is sent as the request body.
    pub fn handle_form_submit(&mut self, data: &crate::forms::FormData, vfs: &dyn Vfs) {
        let encoded = data.encode();
        self.nav.update_scroll(self.scroll.scroll_y);
        let action = &data.action;

        // Resolve the action URL against the current page.
        let resolved_action = if let Some(current) = self.nav.current_url() {
            if let Some(base) = Url::parse(current) {
                base.resolve(action)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| action.to_string())
            } else {
                action.to_string()
            }
        } else {
            action.to_string()
        };

        match data.method {
            crate::forms::FormMethod::Get => {
                // Append form data as query string.
                let url = if encoded.is_empty() {
                    resolved_action
                } else if resolved_action.contains('?') {
                    format!("{resolved_action}&{encoded}")
                } else {
                    format!("{resolved_action}?{encoded}")
                };
                self.visited_urls.insert(url.clone());
                self.navigate_vfs(&url, vfs);
            },
            crate::forms::FormMethod::Post => {
                let body = encoded.into_bytes();
                self.visited_urls.insert(resolved_action.clone());
                self.navigate_post(&resolved_action, body, vfs);
            },
        }
    }
}
