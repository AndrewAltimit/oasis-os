//! Command trait, registry, and dispatch logic.
//!
//! Supports quoted arguments, environment variables, command substitution,
//! command history, pipes, input/output redirection, command chaining,
//! and glob expansion.
//!
//! The implementation is split across these submodules:
//! - [`crate::types`]: core types (`CommandOutput`, `Environment`,
//!   `Command`, `ShellFunction`, limits)
//! - [`crate::registry`]: `CommandRegistry` struct, variable/alias/
//!   function/history APIs
//! - [`crate::expansion`]: history, alias, variable and `$(...)`
//!   expansion of a command into words (on [`crate::expander`]'s scanner)
//! - [`crate::executor`]: execution pipeline (`execute`, lists and
//!   `&`, `!`, pipelines, redirection, `execute_single_cmd`, `run_words`)

// Re-export everything so `use crate::interpreter::*` paths keep working.
pub use crate::expander::resolve_path;
pub use crate::registry::CommandRegistry;
pub use crate::types::{Command, CommandOutput, CommandSignal, Environment};

// Items re-exported only for tests and internal use.
#[cfg(test)]
pub(crate) use crate::expander::case_pattern_matches;
#[cfg(test)]
pub(crate) use crate::expander::expand_braces;
#[cfg(test)]
pub use crate::expander::tokenize;
#[cfg(test)]
pub(crate) use crate::pipeline::parse_redirect;
#[cfg(test)]
use oasis_types::error::Result;

// Builtin commands and script execution are in separate files:
// - builtins.rs: help, which, function, return, break, continue, local,
//                history, set, unset, env, alias, unalias, list_commands,
//                completions
// - jobs.rs:     background jobs (`cmd &`), jobs, fg, bg, kill %N,
//                poll_jobs
// - script.rs:   run, statement splitting, the control-flow parser
//                (if/elif/else, while/until, for, case) and executor
//                shared by scripts and inline one-liners

#[cfg(test)]
mod tests_basic;
#[cfg(test)]
mod tests_pipeline;
#[cfg(test)]
mod tests_script;
#[cfg(test)]
mod tests_semantics;
