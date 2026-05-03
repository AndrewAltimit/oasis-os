//! Top-level widget shell: navigation/loading (`pipeline`), paint dispatch
//! (`paint`), input routing (`input`), and image resource handling (`images`).

pub(crate) mod images;
pub(crate) mod input;
pub(crate) mod paint;
pub(crate) mod pipeline;

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod pipeline_tests;
