//! Shell script parser and executor for [`CommandRegistry`].
//!
//! This is the single control-flow implementation used by both script
//! files (`run <path>`) and interactive one-liners such as
//! `if test -f a; then echo yes; elif test -d a; then echo dir; fi`.
//!
//! Pipeline:
//!
//! 1. [`split_statements`] (`split`) turns source text into logical
//!    statements. It splits on newlines and unquoted `;` (emitting `;;` as
//!    its own statement for `case`), and is aware of quotes, backslash
//!    escapes, `{ ... }` function bodies, `$( ... )` substitutions and `#`
//!    comments.
//! 2. [`parse_script`] (`parse`) builds a [`Node`] tree with arbitrarily
//!    nested `if`/`elif`/`else`/`fi`, `while`/`until`/`do`/`done`,
//!    `for`/`in`/`do`/`done` and `case`/`esac` blocks. Keywords may share a
//!    statement with the following command (`then echo yes`, `do body`,
//!    `else if ...`, `case $x in a) ...`).
//! 3. The executor (`exec`) walks the tree. Simple statements run through
//!    the full chain/pipeline executor, so `&&`, `||`, pipes and
//!    redirection work inside blocks.
//!
//! [`CommandRegistry`]: crate::interpreter::CommandRegistry

mod exec;
mod parse;
mod split;
#[cfg(test)]
mod tests;

pub(crate) use parse::{Node, parse_script};
pub(crate) use split::{Stmt, find_keyword, is_compound, split_statements};
