//! Developer tool commands: base64, json, uuid, seq, expr, test and `[`.
//!
//! - `encoding` -- `base64`, `json` (validate / pretty-print), `uuid`
//! - `arith` -- `seq` and `expr` (with its arithmetic evaluator)
//! - `condition` -- `test` and `[`

use crate::interpreter::CommandRegistry;

mod arith;
mod condition;
mod encoding;
#[cfg(test)]
mod tests;

/// Register the developer tool commands (`base64`, `json`, `uuid`, `seq`,
/// `expr`, `test`, `[`).
pub fn register_dev_commands(reg: &mut CommandRegistry) {
    encoding::register(reg);
    arith::register(reg);
    condition::register(reg);
}
