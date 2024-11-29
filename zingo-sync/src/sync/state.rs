//! Module for creating and manipulating all parts that track the state of the sync engine.

use std::ops::Range;

use zcash_primitives::consensus::BlockHeight;

use crate::{primitives::Locator, traits::SyncWallet};

/// Returns the locators for a given `block_range` from the wallet's [`crate::primitives::SyncState`]
pub(crate) fn find_locators<W>(wallet: &W, block_range: &Range<BlockHeight>) -> Vec<Locator>
where
    W: SyncWallet,
{
    todo!()
}

// TODO: remove locators after range is scanned
