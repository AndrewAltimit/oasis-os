//! Declarative macros for reducing command definition and registration
//! boilerplate.
//!
//! # `define_command!`
//!
//! ```ignore
//! define_command! {
//!     WhoamiCmd, "whoami", "Print current user name", "whoami", "system",
//!     |_args, _env| {
//!         Ok(CommandOutput::Text("oasis".to_string()))
//!     }
//! }
//! ```
//!
//! The macro generates a unit struct and a `Command` trait implementation.
//!
//! # `register_commands!`
//!
//! ```ignore
//! register_commands!(register_dev_commands, [Base64Cmd, JsonCmd, UuidCmd]);
//! ```
//!
//! Generates a `pub fn register_*_commands(reg: &mut CommandRegistry)` that
//! registers each listed command struct.

/// Define a terminal command with minimal boilerplate.
///
/// # Syntax
///
/// ```ignore
/// define_command!(StructName, "name", "description", "usage", "category",
///     |args, env| { /* body */ }
/// );
/// ```
///
/// The closure parameters are `args: &[&str]` and `env: &mut Environment<'_>`.
/// The body must return `Result<CommandOutput>`.
#[macro_export]
macro_rules! define_command {
    (
        $struct:ident, $name:expr, $desc:expr, $usage:expr, $cat:expr,
        |$args:ident, $env:ident| $body:block
    ) => {
        struct $struct;
        impl $crate::Command for $struct {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn usage(&self) -> &str {
                $usage
            }
            fn category(&self) -> &str {
                $cat
            }
            fn execute(
                &self,
                $args: &[&str],
                $env: &mut $crate::Environment<'_>,
            ) -> oasis_types::error::Result<$crate::CommandOutput> {
                $body
            }
        }
    };
}

/// Generate a public registration function that registers command structs.
///
/// # Syntax
///
/// ```ignore
/// register_commands!(register_dev_commands, [Base64Cmd, JsonCmd, UuidCmd]);
/// ```
///
/// Expands to:
/// ```ignore
/// pub fn register_dev_commands(reg: &mut crate::CommandRegistry) {
///     reg.register(Box::new(Base64Cmd));
///     reg.register(Box::new(JsonCmd));
///     reg.register(Box::new(UuidCmd));
/// }
/// ```
#[macro_export]
macro_rules! register_commands {
    ($fn_name:ident, [$($cmd:expr),+ $(,)?]) => {
        pub fn $fn_name(reg: &mut $crate::CommandRegistry) {
            $(reg.register(Box::new($cmd));)+
        }
    };
}
