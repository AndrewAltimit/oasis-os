//! User-agent default stylesheet.
//!
//! Re-exports the default stylesheet from the cascade module.

use super::parser::Stylesheet;

/// Get the user-agent default stylesheet.
pub fn default_stylesheet() -> Stylesheet {
    super::cascade::default_stylesheet()
}
