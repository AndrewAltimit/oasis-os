//! CSS custom property (`var()`) substitution.
//!
//! Declarations that reference `var()` anywhere are stored by the parser
//! as [`CssValue::Unresolved`](crate::css::parser::CssValue) raw text.
//! At computed-value time the cascade calls [`substitute_vars`] to splice
//! in custom property values textually, then re-parses the result with
//! the property's normal parser. A reference that cannot be resolved
//! (missing property with no fallback, or a dependency cycle) makes the
//! whole declaration *invalid at computed-value time*.
//!
//! Custom properties that themselves reference other custom properties
//! are resolved once per element by [`resolve_custom_properties`], so
//! children inherit already-substituted values (CSS Variables §2.2) and
//! cycles invalidate every property in the cycle (§2.3).

use rustc_hash::FxHashMap;

/// Maximum nesting of `var()` resolution (chains, fallbacks-of-fallbacks).
const MAX_VAR_DEPTH: u32 = 32;

/// Upper bound on substituted text, so exponential expansions such as
/// `--b: var(--a) var(--a); --c: var(--b) var(--b); ...` stay bounded.
const MAX_SUBSTITUTED_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
enum SubstError {
    /// Missing reference without fallback, malformed `var()`, or limits
    /// exceeded. `var()` fallbacks apply.
    Invalid,
    /// Part of a dependency cycle rooted at the named property. Fallbacks
    /// do not rescue properties inside the cycle.
    Cycle(String),
}

/// True if `text` contains a `var(` function (ASCII case-insensitive).
pub(super) fn contains_var(text: &str) -> bool {
    text.as_bytes()
        .windows(4)
        .any(|w| w.eq_ignore_ascii_case(b"var("))
}

struct Resolver<'a> {
    props: &'a FxHashMap<String, String>,
    /// Custom properties currently being resolved (cycle detection).
    stack: Vec<String>,
    /// Memoised results for properties that needed substitution.
    memo: FxHashMap<String, Option<String>>,
}

impl<'a> Resolver<'a> {
    fn new(props: &'a FxHashMap<String, String>) -> Self {
        Self {
            props,
            stack: Vec::new(),
            memo: FxHashMap::default(),
        }
    }

    /// The fully substituted value of custom property `name`.
    fn lookup(&mut self, name: &str, depth: u32) -> Result<String, SubstError> {
        if let Some(m) = self.memo.get(name) {
            return m.clone().ok_or(SubstError::Invalid);
        }
        if self.stack.iter().any(|s| s == name) {
            return Err(SubstError::Cycle(name.to_string()));
        }
        let props = self.props;
        let Some(raw) = props.get(name) else {
            return Err(SubstError::Invalid);
        };
        if !contains_var(raw) {
            return Ok(raw.clone());
        }
        self.stack.push(name.to_string());
        let res = self.substitute(raw, depth + 1);
        self.stack.pop();
        let res = match res {
            // This property closed the cycle: it is invalid, and anyone
            // referencing it from outside the cycle may use a fallback.
            Err(SubstError::Cycle(root)) if root == name => Err(SubstError::Invalid),
            other => other,
        };
        self.memo
            .insert(name.to_string(), res.as_ref().ok().cloned());
        res
    }

    /// Replace every `var()` in `text`.
    fn substitute(&mut self, text: &str, depth: u32) -> Result<String, SubstError> {
        if depth > MAX_VAR_DEPTH {
            return Err(SubstError::Invalid);
        }
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut i = 0;
        let mut quote: Option<u8> = None;
        while i < bytes.len() {
            let b = bytes[i];
            if let Some(q) = quote {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == q {
                    quote = None;
                }
                i += 1;
                continue;
            }
            if b == b'"' || b == b'\'' {
                quote = Some(b);
                i += 1;
                continue;
            }
            if !is_var_call(bytes, i) {
                i += 1;
                continue;
            }
            let args_start = i + 4;
            let close = matching_paren(bytes, args_start).ok_or(SubstError::Invalid)?;
            out.push_str(&text[last..i]);
            let args = &text[args_start..close];
            let (name, fallback) = match top_level_comma(args.as_bytes()) {
                Some(c) => (args[..c].trim(), Some(args[c + 1..].trim())),
                None => (args.trim(), None),
            };
            if !name.starts_with("--") {
                return Err(SubstError::Invalid);
            }
            let value = match self.lookup(name, depth) {
                Ok(v) => v,
                Err(SubstError::Invalid) => match fallback {
                    Some(fb) => self.substitute(fb, depth + 1)?,
                    None => return Err(SubstError::Invalid),
                },
                Err(cycle) => return Err(cycle),
            };
            out.push_str(&value);
            if out.len() > MAX_SUBSTITUTED_LEN {
                return Err(SubstError::Invalid);
            }
            i = close + 1;
            last = i;
        }
        out.push_str(&text[last..]);
        Ok(out)
    }
}

/// `var(` starting at `i`, not preceded by an identifier character.
fn is_var_call(bytes: &[u8], i: usize) -> bool {
    if i + 4 > bytes.len() || !bytes[i..i + 4].eq_ignore_ascii_case(b"var(") {
        return false;
    }
    i == 0 || !matches!(bytes[i - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_')
}

/// Index of the `)` closing a group whose contents start at `start`.
fn matching_paren(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1u32;
    let mut quote: Option<u8> = None;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(_) if b == b'\\' => i += 1,
            Some(q) if b == q => quote = None,
            Some(_) => {},
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                },
                _ => {},
            },
        }
        i += 1;
    }
    None
}

/// Index of the first comma outside nested parentheses.
fn top_level_comma(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0u32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return Some(i),
            _ => {},
        }
    }
    None
}

/// Substitute every `var()` reference in a declaration value.
///
/// Returns `None` when the declaration is invalid at computed-value time.
pub(super) fn substitute_vars(text: &str, props: &FxHashMap<String, String>) -> Option<String> {
    Resolver::new(props).substitute(text, 0).ok()
}

/// Resolve `var()` references inside the custom properties declared on
/// this element (`declared`), in place. Properties that are invalid at
/// computed-value time — including every member of a reference cycle —
/// are removed, i.e. they take the guaranteed-invalid initial value.
pub(super) fn resolve_custom_properties(props: &mut FxHashMap<String, String>, declared: &[&str]) {
    if !declared
        .iter()
        .any(|n| props.get(*n).is_some_and(|v| contains_var(v)))
    {
        return;
    }
    let mut updates: Vec<(String, Option<String>)> = Vec::new();
    {
        let mut resolver = Resolver::new(props);
        for &name in declared {
            if props.get(name).is_some_and(|v| contains_var(v)) {
                let value = resolver.lookup(name, 0).ok();
                updates.push((name.to_string(), value));
            }
        }
    }
    for (name, value) in updates {
        match value {
            Some(v) => {
                props.insert(name, v);
            },
            None => {
                props.remove(&name);
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, &str)]) -> FxHashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn substitutes_inside_functions() {
        let p = props(&[("--gap", "8px"), ("--r", "255")]);
        assert_eq!(
            substitute_vars("calc(var(--gap) * 2)", &p).as_deref(),
            Some("calc(8px * 2)")
        );
        assert_eq!(
            substitute_vars("rgb(var(--r), 0, 0)", &p).as_deref(),
            Some("rgb(255, 0, 0)")
        );
    }

    #[test]
    fn nested_fallbacks() {
        let p = props(&[("--c", "blue")]);
        assert_eq!(
            substitute_vars("var(--a, var(--b, var(--c)))", &p).as_deref(),
            Some("blue")
        );
        assert_eq!(substitute_vars("var(--a, var(--b))", &p), None);
        assert_eq!(
            substitute_vars("var(--a, 1px 2px)", &p).as_deref(),
            Some("1px 2px")
        );
    }

    #[test]
    fn names_are_case_sensitive() {
        let p = props(&[("--Foo", "red")]);
        assert_eq!(substitute_vars("var(--Foo)", &p).as_deref(), Some("red"));
        assert_eq!(substitute_vars("var(--foo)", &p), None);
    }

    #[test]
    fn cycles_invalidate_members_but_not_referrers_fallbacks() {
        let mut p = props(&[
            ("--a", "var(--b, 1px)"),
            ("--b", "var(--a, 2px)"),
            ("--c", "var(--a, 3px)"),
        ]);
        resolve_custom_properties(&mut p, &["--a", "--b", "--c"]);
        assert!(!p.contains_key("--a"));
        assert!(!p.contains_key("--b"));
        assert_eq!(p.get("--c").map(String::as_str), Some("3px"));
    }

    #[test]
    fn exponential_expansion_is_bounded() {
        let mut pairs = vec![("--v0".to_string(), "xxxxxxxxxxxxxxxx".to_string())];
        for i in 1..30 {
            pairs.push((format!("--v{i}"), format!("var(--v{0}) var(--v{0})", i - 1)));
        }
        let p: FxHashMap<String, String> = pairs.into_iter().collect();
        assert_eq!(substitute_vars("var(--v29)", &p), None);
    }

    #[test]
    fn var_inside_strings_is_left_alone() {
        let p = props(&[("--x", "1")]);
        assert_eq!(
            substitute_vars("\"var(--x)\" var(--x)", &p).as_deref(),
            Some("\"var(--x)\" 1")
        );
    }
}
