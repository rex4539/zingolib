//! Traits for interfacing a wallet with the sync engine

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;

use incrementalmerkletree::Level;
use shardtree::LocatedPrunableTree;
use zcash_client_backend::{
    data_api::{ORCHARD_SHARD_HEIGHT, SAPLING_SHARD_HEIGHT},
    keys::UnifiedFullViewingKey,
};
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::AccountId;

use crate::keys::transparent::TransparentAddressId;
use crate::primitives::{NullifierMap, OutPointMap, SyncState, WalletBlock, WalletTransaction};
use crate::witness::{ShardTreeData, ShardTrees};

// TODO: clean up interface and move many default impls out of traits. consider merging to a simplified SyncWallet interface.

/// Temporary dump for all neccessary wallet functionality for PoC
pub trait SyncWallet {
    /// Errors associated with interfacing the sync engine with wallet data
    type Error: Debug;

    /// Returns the block height wallet was created.
    fn get_birthday(&self) -> Result<BlockHeight, Self::Error>;

    /// Returns a reference to wallet sync state.
    fn get_sync_state(&self) -> Result<&SyncState, Self::Error>;

    /// Returns a mutable reference to wallet sync state.
    fn get_sync_state_mut(&mut self) -> Result<&mut SyncState, Self::Error>;

    /// Returns all unified full viewing keys known to this wallet.
    fn get_unified_full_viewing_keys(
        &self,
    ) -> Result<HashMap<AccountId, UnifiedFullViewingKey>, Self::Error>;

    /// Returns a reference to all the transparent addresses known to this wallet.
    fn get_transparent_addresses(
        &self,
    ) -> Result<&BTreeMap<TransparentAddressId, String>, Self::Error>;

    /// Returns a mutable reference to all the transparent addresses known to this wallet.
    fn get_transparent_addresses_mut(
        &mut self,
    ) -> Result<&mut BTreeMap<TransparentAddressId, String>, Self::Error>;
}

/// Trait for interfacing [`crate::primitives::WalletBlock`]s with wallet data
pub trait SyncBlocks: SyncWallet {
    /// Get a stored wallet compact block from wallet data by block height
    /// Must return error if block is not found
    fn get_wallet_block(&self, block_height: BlockHeight) -> Result<WalletBlock, Self::Error>;

    /// Get mutable reference to wallet blocks
    fn get_wallet_blocks_mut(
        &mut self,
    ) -> Result<&mut BTreeMap<BlockHeight, WalletBlock>, Self::Error>;

    /// Append wallet compact blocks to wallet data
    fn append_wallet_blocks(
        &mut self,
        mut wallet_blocks: BTreeMap<BlockHeight, WalletBlock>,
    ) -> Result<(), Self::Error> {
        self.get_wallet_blocks_mut()?.append(&mut wallet_blocks);

        Ok(())
    }

    /// Removes all wallet blocks above the given `block_height`.
    fn truncate_wallet_blocks(&mut self, truncate_height: BlockHeight) -> Result<(), Self::Error> {
        self.get_wallet_blocks_mut()?
            .retain(|block_height, _| *block_height <= truncate_height);

        Ok(())
    }
}

/// Trait for interfacing [`crate::primitives::WalletTransaction`]s with wallet data
pub trait SyncTransactions: SyncWallet {
    /// Get reference to wallet transactions
    fn get_wallet_transactions(&self) -> Result<&HashMap<TxId, WalletTransaction>, Self::Error>;

    /// Get mutable reference to wallet transactions
    fn get_wallet_transactions_mut(
        &mut self,
    ) -> Result<&mut HashMap<TxId, WalletTransaction>, Self::Error>;

    /// Extend wallet transaction map with new wallet transactions
    fn extend_wallet_transactions(
        &mut self,
        wallet_transactions: HashMap<TxId, WalletTransaction>,
    ) -> Result<(), Self::Error> {
        self.get_wallet_transactions_mut()?
            .extend(wallet_transactions);

        Ok(())
    }

    /// Removes all wallet transactions above the given `block_height`.
    /// Also sets any output's spending_transaction field to `None` if it's spending transaction was removed.
    fn truncate_wallet_transactions(
        &mut self,
        truncate_height: BlockHeight,
    ) -> Result<(), Self::Error> {
        // TODO: Replace with `extract_if()` when it's in stable rust
        let invalid_txids: Vec<TxId> = self
            .get_wallet_transactions()?
            .values()
            .filter(|tx| tx.block_height() > truncate_height)
            .map(|tx| tx.transaction().txid())
            .collect();

        let wallet_transactions = self.get_wallet_transactions_mut()?;
        wallet_transactions
            .values_mut()
            .flat_map(|tx| tx.sapling_notes_mut())
            .filter(|note| {
                note.spending_transaction().map_or_else(
                    || false,
                    |spending_txid| invalid_txids.contains(&spending_txid),
                )
            })
            .for_each(|note| {
                note.set_spending_transaction(None);
            });
        wallet_transactions
            .values_mut()
            .flat_map(|tx| tx.orchard_notes_mut())
            .filter(|note| {
                note.spending_transaction().map_or_else(
                    || false,
                    |spending_txid| invalid_txids.contains(&spending_txid),
                )
            })
            .for_each(|note| {
                note.set_spending_transaction(None);
            });

        invalid_txids.iter().for_each(|invalid_txid| {
            wallet_transactions.remove(invalid_txid);
        });

        Ok(())
    }
}

/// Trait for interfacing nullifiers with wallet data
pub trait SyncNullifiers: SyncWallet {
    /// Get wallet nullifier map
    fn get_nullifiers(&self) -> Result<&NullifierMap, Self::Error>;

    /// Get mutable reference to wallet nullifier map
    fn get_nullifiers_mut(&mut self) -> Result<&mut NullifierMap, Self::Error>;

    /// Append nullifiers to wallet nullifier map
    fn append_nullifiers(&mut self, mut nullifier_map: NullifierMap) -> Result<(), Self::Error> {
        self.get_nullifiers_mut()?
            .sapling_mut()
            .append(nullifier_map.sapling_mut());
        self.get_nullifiers_mut()?
            .orchard_mut()
            .append(nullifier_map.orchard_mut());

        Ok(())
    }

    /// Removes all mapped nullifiers above the given `block_height`.
    fn truncate_nullifiers(&mut self, truncate_height: BlockHeight) -> Result<(), Self::Error> {
        let nullifier_map = self.get_nullifiers_mut()?;
        nullifier_map
            .sapling_mut()
            .retain(|_, (block_height, _)| *block_height <= truncate_height);
        nullifier_map
            .orchard_mut()
            .retain(|_, (block_height, _)| *block_height <= truncate_height);

        Ok(())
    }
}

/// Trait for interfacing outpoints with wallet data
pub trait SyncOutPoints: SyncWallet {
    /// Get wallet outpoint map
    fn get_outpoints(&self) -> Result<&OutPointMap, Self::Error>;

    /// Get mutable reference to wallet outpoint map
    fn get_outpoints_mut(&mut self) -> Result<&mut OutPointMap, Self::Error>;

    /// Append outpoints to wallet outpoint map
    fn append_outpoints(&mut self, mut outpoint_map: OutPointMap) -> Result<(), Self::Error> {
        self.get_outpoints_mut()?
            .inner_mut()
            .append(outpoint_map.inner_mut());

        Ok(())
    }

    /// Removes all mapped outpoints above the given `block_height`.
    fn truncate_outpoints(&mut self, truncate_height: BlockHeight) -> Result<(), Self::Error> {
        self.get_outpoints_mut()?
            .inner_mut()
            .retain(|_, (block_height, _)| *block_height <= truncate_height);

        Ok(())
    }
}

/// Trait for interfacing shard tree data with wallet data
pub trait SyncShardTrees: SyncWallet {
    /// Get mutable reference to shard trees
    fn get_shard_trees_mut(&mut self) -> Result<&mut ShardTrees, Self::Error>;

    /// Update wallet shard trees with new shard tree data
    fn update_shard_trees(&mut self, shard_tree_data: ShardTreeData) -> Result<(), Self::Error> {
        let ShardTreeData {
            sapling_initial_position,
            orchard_initial_position,
            sapling_leaves_and_retentions,
            orchard_leaves_and_retentions,
        } = shard_tree_data;
        //TODO: Play with numbers. Is it more efficient to
        // build larger trees to allow for more pruning to
        // happen in parallel before insertion?
        // Is it better to build smaller trees so that more
        // trees can be built in parallel at the same time?
        // Is inserting trees more efficient if trees are
        // a power of 2 size? Is it more efficient if they
        // are 'aligned' so that the initial_position is
        // a multiple of tree size? All unanswered questions
        // that want to be benchmarked.

        let (sapling_sender, sapling_receiver) = crossbeam_channel::unbounded();
        let (orchard_sender, orchard_receiver) = crossbeam_channel::unbounded();
        rayon::scope_fifo(|scope| {
            for (i, sapling_chunk) in sapling_leaves_and_retentions.chunks(128).enumerate() {
                let sapling_sender = sapling_sender.clone();
                scope.spawn_fifo(move |_scope| {
                    let start_position = sapling_initial_position + (i as u64 * 128);
                    let tree = LocatedPrunableTree::from_iter(
                        start_position..(start_position + sapling_chunk.len() as u64),
                        Level::from(SAPLING_SHARD_HEIGHT),
                        sapling_chunk.iter().copied(),
                    );
                    sapling_sender.send(tree).unwrap();
                })
            }

            for (i, orchard_chunk) in orchard_leaves_and_retentions.chunks(128).enumerate() {
                let orchard_sender = orchard_sender.clone();
                scope.spawn_fifo(move |_scope| {
                    let start_position = orchard_initial_position + (i as u64 * 128);
                    let tree = LocatedPrunableTree::from_iter(
                        start_position..(start_position + orchard_chunk.len() as u64),
                        Level::from(ORCHARD_SHARD_HEIGHT),
                        orchard_chunk.iter().copied(),
                    );
                    orchard_sender.send(tree).unwrap();
                })
            }
        });
        drop((orchard_sender, sapling_sender));

        let trees = self.get_shard_trees_mut()?;
        for tree in sapling_receiver.iter() {
            let tree = tree.unwrap();
            trees
                .sapling_mut()
                .insert_tree(tree.subtree, tree.checkpoints)
                .unwrap();
        }
        for tree in orchard_receiver {
            let tree = tree.unwrap();
            trees
                .orchard_mut()
                .insert_tree(tree.subtree, tree.checkpoints)
                .unwrap();
        }

        Ok(())
    }

    /// Removes all shard tree data above the given `block_height`.
    fn truncate_shard_trees(&mut self, truncate_height: BlockHeight) -> Result<(), Self::Error> {
        // TODO: investigate resetting the shard completely when truncate height is 0
        if !self
            .get_shard_trees_mut()?
            .sapling_mut()
            .truncate_to_checkpoint(&truncate_height)
            .unwrap()
        {
            panic!("max checkpoints should always be higher than verification window!");
        }
        if !self
            .get_shard_trees_mut()?
            .orchard_mut()
            .truncate_to_checkpoint(&truncate_height)
            .unwrap()
        {
            panic!("max checkpoints should always be higher than verification window!");
        }

        Ok(())
    }
}
