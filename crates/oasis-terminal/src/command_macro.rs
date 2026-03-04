//! Declarative macro for reducing command definition boilerplate.
//!
//! # Usage
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
