#![warn(missing_docs)]
//! Zingo sync engine prototype
//!
//! Entrypoint: [`crate::sync::sync`]

pub mod client;
pub mod error;
pub mod keys;
#[allow(missing_docs)]
pub mod primitives;
pub(crate) mod scan;
pub mod sync;
pub mod traits;
pub(crate) mod utils;
pub mod witness;
