//! Command trait, registry, and dispatch logic.
//!
//! Supports quoted arguments, environment variables, command substitution,
//! command history, pipes, input/output redirection, command chaining,
//! and glob expansion.
//!
//! The implementation is split across three submodules:
//! - [`crate::types`]: core types (`CommandOutput`, `Environment`,
//!   `Command`, `ShellFunction`, constants)
//! - [`crate::registry`]: `CommandRegistry` struct, variable/alias/
//!   function/history APIs, expansion helpers
//! - [`crate::executor`]: execution pipeline (`execute`,
//!   `execute_pipeline`, `execute_with_redirect`,
//!   `execute_single_cmd`, `expand_substitutions`)

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
// - script.rs:   run, execute_script_block, collect_loop_body,
//                execute_if_block, execute_case_block,
//                execute_script_line, eval_condition

#[cfg(test)]
mod tests_basic;
#[cfg(test)]
mod tests_pipeline;
#[cfg(test)]
mod tests_script;
