#![warn(missing_docs)]
//! Zingo sync engine prototype
//!
//! Definitions, Subordinate to the [Zcash Protocol Specification](<https://zips.z.cash/protocol/protocol-dark.pdf#blockchain>):
//!  - Sync: Observation of a consensus state
//!  - Consensus State:  Eventually consistent global agreement
//!  - Key-Enabled (keyed): Sync where the observer owns keys that reveal hidden information
//!  - Keyless: The observer doesn't have keys
//!
//! Entrypoint: [`crate::sync::sync`]

pub mod client;
pub mod error;
pub(crate) mod keys;
#[allow(missing_docs)]
pub mod primitives;
pub(crate) mod scan;
pub mod sync;
pub mod traits;
pub(crate) mod utils;
pub mod witness;
