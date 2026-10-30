//! Expansion of a simple command into words, on [`CommandRegistry`].
//!
//! Order, per simple command:
//!
//! 1. history (`!!`, `!n`; whole line, at the top of `execute`),
//! 2. alias substitution of the unquoted first word (raw text, one level),
//! 3. one left-to-right pass of [`split_words`] doing quote removal and
//!    `$VAR` / `${VAR}` / `$?` / `$#` / `$(...)` substitution. A value is
//!    inserted *as word text*: unquoted it is split into fields on
//!    whitespace, quoted it is kept whole, and in neither case is it
//!    parsed again (quotes, `$`, `\` and `;` in a value are plain
//!    characters; `$(echo '$HOME')` yields `$HOME`, not the variable),
//! 4. brace expansion of plain unquoted words, then pathname (glob)
//!    expansion of words holding an unquoted `*` / `?` / `[` (see
//!    [`expand_words`]).

use std::collections::HashMap;

use oasis_types::error::{OasisError, Result};

use crate::expander::{Word, expand_words, split_words};
use crate::pipeline::output_to_text;
use crate::registry::CommandRegistry;
use crate::types::{Environment, MAX_EXPANDED_LEN};

/// The error for an expansion over [`MAX_EXPANDED_LEN`].
pub(crate) fn expansion_too_long() -> OasisError {
    OasisError::Command(format!("expansion too long (over {MAX_EXPANDED_LEN} bytes)").into())
}

/// Index of the `)` closing a `$(` whose body starts at `start`, or `None`
/// when it is unclosed. Quotes and backslash escapes are honoured and every
/// `(` nests, matching [`crate::pipeline::scan`].
pub(crate) fn find_subst_end(chars: &[char], start: usize) -> Option<usize> {
    let (mut depth, mut j) = (1usize, start);
    let (mut in_sq, mut in_dq) = (false, false);
    while j < chars.len() {
        let c = chars[j];
        if in_sq {
            in_sq = c != '\'';
        } else if in_dq {
            match c {
                '"' => in_dq = false,
                '\\' => j += 1,
                _ => {},
            }
        } else {
            match c {
                '\'' => in_sq = true,
                '"' => in_dq = true,
                '\\' => j += 1,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                },
                _ => {},
            }
        }
        j += 1;
    }
    None
}

/// Which parameter a `$` at some index names, and where it ends.
enum Dollar {
    /// `$NAME`, `${NAME}`, `$?`, `$#`.
    Var(String),
    /// `$(...)`: the inner command text.
    Subst(String),
}

/// Parse the expansion starting at the `$` at `chars[i]`. Returns it and
/// the index just past it, or `None` for a literal `$`.
fn parse_dollar(chars: &[char], i: usize) -> Option<(Dollar, usize)> {
    match *chars.get(i + 1)? {
        c @ ('?' | '#') => Some((Dollar::Var(c.to_string()), i + 2)),
        '{' => {
            let len = chars[i + 2..].iter().position(|&c| c == '}')?;
            let name = chars[i + 2..i + 2 + len].iter().collect();
            Some((Dollar::Var(name), i + 3 + len))
        },
        '(' => {
            let close = find_subst_end(chars, i + 2)?;
            let inner = chars[i + 2..close].iter().collect();
            Some((Dollar::Subst(inner), close + 1))
        },
        c if c.is_alphanumeric() || c == '_' => {
            let end = chars[i + 1..]
                .iter()
                .position(|&c| !(c.is_alphanumeric() || c == '_'))
                .map_or(chars.len(), |n| i + 1 + n);
            Some((Dollar::Var(chars[i + 1..end].iter().collect()), end))
        },
        _ => None,
    }
}

impl CommandRegistry {
    /// Expand `input` (one simple command, or a `for` / `case` word list)
    /// into words: quote removal plus variable and `$(...)` substitution in
    /// one pass. No alias, brace or glob expansion (see
    /// [`expand_args`](Self::expand_args)). Fails when the words would
    /// exceed 256 KiB.
    #[inline(never)]
    pub(crate) fn expand_words_of(
        &self,
        input: &str,
        env: &mut Environment<'_>,
    ) -> Result<Vec<Word>> {
        split_words(input, &mut |chars, i, in_double, out| {
            let Some((dollar, end)) = parse_dollar(chars, i) else {
                return Ok(None);
            };
            match dollar {
                Dollar::Var(name) => {
                    out.push_expansion(&self.var_value(&name, &env.cwd), in_double)
                },
                Dollar::Subst(inner) => {
                    let text = self.substitute(&inner, env);
                    out.push_expansion(text.trim_end_matches('\n'), in_double);
                },
            }
            if out.len() > MAX_EXPANDED_LEN {
                return Err(expansion_too_long());
            }
            Ok(Some(end))
        })
    }

    /// Fully expand a simple command's text into argument strings: the
    /// alias of the first word, [`expand_words_of`](Self::expand_words_of),
    /// then braces and globs.
    pub(crate) fn expand_args(
        &self,
        input: &str,
        env: &mut Environment<'_>,
    ) -> Result<Vec<String>> {
        let aliased = self.apply_alias(input);
        let words = self.expand_words_of(aliased.as_deref().unwrap_or(input), env)?;
        Ok(expand_words(words, env.vfs, &env.cwd))
    }

    /// Run the body of a `$(...)` and return what it printed. Errors are
    /// reported on stderr and substitute nothing. The pipeline's stdin is
    /// not visible to the inner command.
    fn substitute(&self, inner: &str, env: &mut Environment<'_>) -> String {
        let stdin = env.stdin.take();
        let text = match self.execute(inner, env) {
            Ok(out) => output_to_text(&out),
            Err(e) => {
                env.stderr.push_str(&format!("command substitution: {e}"));
                String::new()
            },
        };
        env.stdin = stdin;
        text
    }

    /// The value of shell parameter `name` (empty when unset). `CWD` is the
    /// working directory; `?` the status of the previous command.
    pub(crate) fn var_value(&self, name: &str, cwd: &str) -> String {
        let vars = self.variables.borrow();
        resolve_var(name, &vars, cwd, self.last_exit_code.get())
    }

    /// If the first word of `cmd` is plain unquoted text naming an alias,
    /// the command with that word replaced by the alias text (expanded
    /// along with the rest of the line afterwards; one level only).
    pub(crate) fn apply_alias(&self, cmd: &str) -> Option<String> {
        let cmd = cmd.trim_start();
        let end = cmd.find(char::is_whitespace).unwrap_or(cmd.len());
        let first = &cmd[..end];
        if first.is_empty() || first.contains(['\'', '"', '\\', '$']) {
            return None;
        }
        let aliases = self.aliases.borrow();
        let expansion = aliases.get(first)?;
        Some(format!("{expansion}{}", &cmd[end..]))
    }

    /// Replace the parameter expansions (`$VAR`, `${VAR}`, `$?`, `$#`,
    /// `$1`...) in `text` with their current values, quoted so the result
    /// reads back as the same words. `$(...)` is left in place.
    ///
    /// A background job runs later, at top level; freezing its parameters
    /// when it is queued gives it the values of the moment it was started
    /// (`for i in 1 2; do echo $i & done` prints 1 and 2, not 2 twice).
    pub(crate) fn freeze_parameters(&self, text: &str, cwd: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let (mut in_single, mut in_double) = (false, false);
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if in_single {
                in_single = c != '\'';
                out.push(c);
                i += 1;
                continue;
            }
            match c {
                '\'' if !in_double => in_single = true,
                '"' => in_double = !in_double,
                '\\' => {
                    out.push(c);
                    out.extend(chars.get(i + 1));
                    i += 2;
                    continue;
                },
                '$' => match parse_dollar(&chars, i) {
                    Some((Dollar::Var(name), end)) => {
                        let value = self.var_value(&name, cwd);
                        quote_into(&mut out, &value, in_double);
                        i = end;
                        continue;
                    },
                    Some((Dollar::Subst(_), end)) => {
                        out.extend(&chars[i..end]);
                        i = end;
                        continue;
                    },
                    None => {},
                },
                _ => {},
            }
            out.push(c);
            i += 1;
        }
        out
    }

    // -- History expansion --

    /// `!!` (the previous line) and `!n` (history entry n), whole line only.
    pub(crate) fn expand_history(&self, input: &str) -> Result<String> {
        if input == "!!" {
            let hist = self.history.borrow();
            return hist
                .last()
                .cloned()
                .ok_or_else(|| OasisError::Command("!!: no previous command".into()));
        }
        if let Some(n_str) = input.strip_prefix('!')
            && let Ok(n) = n_str.parse::<usize>()
        {
            let hist = self.history.borrow();
            if n == 0 || n > hist.len() {
                return Err(OasisError::Command(format!("!{n}: event not found").into()));
            }
            return Ok(hist[n - 1].clone());
        }
        Ok(input.to_string())
    }

    /// Test helper: expand `input` and join the words with spaces.
    #[cfg(test)]
    pub(crate) fn expand_joined(&self, input: &str, env: &mut Environment<'_>) -> Result<String> {
        let words = self.expand_words_of(input, env)?;
        Ok(words
            .into_iter()
            .map(|w| w.text)
            .collect::<Vec<_>>()
            .join(" "))
    }
}

fn resolve_var(name: &str, vars: &HashMap<String, String>, cwd: &str, last: i32) -> String {
    match name {
        "CWD" => cwd.to_string(),
        // The status of the previous command (the chain executor updates
        // `?` once a pipeline finishes).
        "?" => vars.get("?").cloned().unwrap_or_else(|| last.to_string()),
        _ => vars.get(name).cloned().unwrap_or_default(),
    }
}

/// Append `value` to `out` so the tokenizer reads it back verbatim: inside
/// double quotes `"`, `\` and `$` are escaped; unquoted, every character
/// that is not plainly literal is escaped except whitespace (which still
/// splits fields, as the unquoted expansion would have).
fn quote_into(out: &mut String, value: &str, in_double: bool) {
    for c in value.chars() {
        let escape = if in_double {
            matches!(c, '"' | '\\' | '$')
        } else {
            !(c.is_alphanumeric()
                || c.is_whitespace()
                || matches!(c, '.' | '/' | '-' | '_' | ',' | ':' | '=' | '+' | '@' | '%'))
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_env;
    use oasis_vfs::MemoryVfs;

    #[test]
    fn find_subst_end_nests_and_skips_quotes() {
        let chars: Vec<char> = "echo (a) ')' \")\" $(b))x".chars().collect();
        let end = find_subst_end(&chars, 0).unwrap();
        assert_eq!(chars[end + 1], 'x');
        assert!(find_subst_end(&"echo (".chars().collect::<Vec<_>>(), 0).is_none());
    }

    #[test]
    fn unquoted_value_is_field_split_quoted_is_not() {
        let reg = CommandRegistry::new();
        reg.set_variable("V", "a  b");
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let words = reg.expand_words_of("x $V \"$V\"", &mut env).unwrap();
        let texts: Vec<_> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["x", "a", "b", "a  b"]);
    }

    #[test]
    fn values_are_not_reparsed() {
        let reg = CommandRegistry::new();
        reg.set_variable("Q", "'it''s' \"x\" \\n $HOME ;");
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        let words = reg.expand_words_of("$Q", &mut env).unwrap();
        let texts: Vec<_> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["'it''s'", "\"x\"", "\\n", "$HOME", ";"]);
    }

    #[test]
    fn empty_unquoted_expansion_vanishes_quoted_stays() {
        let reg = CommandRegistry::new();
        let mut vfs = MemoryVfs::new();
        let mut env = make_env(&mut vfs);
        assert!(reg.expand_words_of("$NOPE", &mut env).unwrap().is_empty());
        let words = reg.expand_words_of("\"$NOPE\"", &mut env).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "");
    }

    #[test]
    fn freeze_parameters_quotes_values() {
        let reg = CommandRegistry::new();
        reg.set_variable("X", "a b;c");
        let frozen = reg.freeze_parameters("echo $X \"$X\" '$X' $(echo $X)", "/");
        assert_eq!(frozen, "echo a b\\;c \"a b;c\" '$X' $(echo $X)");
    }

    #[test]
    fn alias_applies_to_plain_first_word_only() {
        let reg = CommandRegistry::new();
        reg.set_alias("ll", "ls -l");
        assert_eq!(reg.apply_alias("ll /tmp").as_deref(), Some("ls -l /tmp"));
        assert_eq!(reg.apply_alias("'ll' /tmp"), None);
        assert_eq!(reg.apply_alias("echo ll"), None);
    }
}
