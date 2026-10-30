//! CSS tokenizer, parser, cascade and computed values for the OASIS_OS
//! browser.
//!
//! Parses stylesheets ([`parser::Stylesheet`]), runs the cascade over an
//! [`oasis_html::dom::Document`] ([`cascade::style_tree`]) and produces a
//! [`values::ComputedStyle`] per element, plus the keyframe animation and
//! transition timelines. Depends only on `oasis-html` and `oasis-types`;
//! layout-dependent pieces (the `@container` size-lookup walk, the
//! transform matrix math) live in `oasis-browser`.
//!
//! `oasis-browser` re-exports this crate as its internal `css` module;
//! embedders normally reach it through `oasis_browser::internals`. The
//! API is shaped for the browser engine and may change without notice.
//!
//! # Feature flags
//!
//! - `parallel-style` — parallel style cascade via rayon (desktop only).
//! - `web-fonts` — adds `ComputedStyle::web_font_id` for `@font-face`
//!   matching (forwarded from `oasis-browser/web-fonts`).
//! - `scripting` — the UA stylesheet treats scripting as enabled, so
//!   `<noscript>` stays hidden (forwarded from `oasis-browser/javascript`).

pub mod animation;
pub mod cascade;
mod color;
pub mod default;
mod media;
pub mod parser;
pub mod selectors;
mod shorthand;
mod token_util;
pub mod tokenizer;
pub mod transition;
pub mod values;
