//! Entrypoint for sync engine

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use crate::client::{self, FetchRequest};
use crate::error::SyncError;
use crate::keys::transparent::TransparentAddressId;
use crate::primitives::{Locator, NullifierMap, OutPointMap, OutputId, WalletTransaction};
use crate::scan::error::{ContinuityError, ScanError};
use crate::scan::task::{Scanner, ScannerState};
use crate::scan::transactions::{scan_transaction, scan_transactions};
use crate::scan::{DecryptedNoteData, ScanResults};
use crate::traits::{
    SyncBlocks, SyncNullifiers, SyncOutPoints, SyncShardTrees, SyncTransactions, SyncWallet,
};

use zcash_client_backend::proto::service::RawTransaction;
use zcash_client_backend::{
    data_api::scanning::{ScanPriority, ScanRange},
    proto::service::compact_tx_streamer_client::CompactTxStreamerClient,
};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::{self, BlockHeight};

use tokio::sync::mpsc;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::AccountId;
use zingo_status::confirmation_status::ConfirmationStatus;

pub(crate) mod state;
pub(crate) mod transparent;

// TODO; replace fixed batches with orchard shard ranges (block ranges containing all note commitments to an orchard shard or fragment of a shard)
const BATCH_SIZE: u32 = 1_000;
const VERIFY_BLOCK_RANGE_SIZE: u32 = 10;
const MAX_VERIFICATION_WINDOW: u32 = 100; // TODO: fail if re-org goes beyond this window

/// Syncs a wallet to the latest state of the blockchain
pub async fn sync<P, W>(
    client: CompactTxStreamerClient<zingo_netutils::UnderlyingService>, // TODO: change underlying service for generic
    consensus_parameters: &P,
    wallet: &mut W,
) -> Result<(), SyncError>
where
    P: consensus::Parameters + Sync + Send + 'static,
    W: SyncWallet + SyncBlocks + SyncTransactions + SyncNullifiers + SyncOutPoints + SyncShardTrees,
{
    tracing::info!("Syncing wallet...");

    // create channel for sending fetch requests and launch fetcher task
    let (fetch_request_sender, fetch_request_receiver) = mpsc::unbounded_channel();
    let fetcher_handle = tokio::spawn(client::fetch::fetch(
        fetch_request_receiver,
        client,
        consensus_parameters.clone(),
    ));

    let wallet_height = state::get_wallet_height(consensus_parameters, wallet).unwrap();
    let chain_height = client::get_chain_height(fetch_request_sender.clone())
        .await
        .unwrap();
    if wallet_height > chain_height {
        if wallet_height - chain_height > MAX_VERIFICATION_WINDOW {
            panic!(
                "wallet height is more than {} blocks ahead of best chain height!",
                MAX_VERIFICATION_WINDOW
            );
        }
        truncate_wallet_data(wallet, chain_height).unwrap();
    }
    let ufvks = wallet.get_unified_full_viewing_keys().unwrap();

    transparent::update_addresses_and_locators(
        consensus_parameters,
        wallet,
        fetch_request_sender.clone(),
        &ufvks,
        wallet_height,
        chain_height,
    )
    .await;

    state::update_scan_ranges(
        wallet_height,
        chain_height,
        wallet.get_sync_state_mut().unwrap(),
    )
    .await
    .unwrap();

    // create channel for receiving scan results and launch scanner
    let (scan_results_sender, mut scan_results_receiver) = mpsc::unbounded_channel();
    let mut scanner = Scanner::new(
        consensus_parameters.clone(),
        scan_results_sender,
        fetch_request_sender.clone(),
        ufvks.clone(),
    );
    scanner.spawn_workers();

    // setup the initial mempool stream
    let mut mempool_stream = client::get_mempool_transaction_stream(fetch_request_sender.clone())
        .await
        .unwrap();

    // TODO: consider what happens when there is no verification range i.e. all ranges already scanned
    // TODO: invalidate any pending transactions after eviction height (40 below best chain height?)

    let mut interval = tokio::time::interval(Duration::from_millis(30));
    loop {
        tokio::select! {
            Some((scan_range, scan_results)) = scan_results_receiver.recv() => {
                process_scan_results(
                    consensus_parameters,
                    wallet,
                    fetch_request_sender.clone(),
                    &ufvks,
                    scan_range,
                    scan_results,
                    scanner.state_mut(),
                )
                .await
                .unwrap();
            }

            mempool_stream_response = mempool_stream.message() => {
                process_mempool_stream_response(
                    consensus_parameters,
                    fetch_request_sender.clone(),
                    &ufvks,
                    wallet,
                    mempool_stream_response,
                    &mut mempool_stream)
                .await;

                // reset interval to ensure all mempool transactions have been scanned before sync completes.
                // if a full interval passes without receiving a transaction from the mempool we can safely finish sync.
                interval.reset();
            }

            _update_scanner = interval.tick() => {
                scanner.update(wallet).await;

                if sync_complete(&scanner, &scan_results_receiver, wallet) {
                    tracing::info!("Sync complete.");
                    break;
                }
            }
        }
    }

    drop(scanner);
    drop(fetch_request_sender);
    fetcher_handle.await.unwrap().unwrap();

    Ok(())
}

/// Returns true if sync is complete.
///
/// Sync is complete when:
/// - all scan workers have been shutdown
/// - there is no unprocessed scan results in the channel
/// - all scan ranges have `Scanned` priority
fn sync_complete<P, W>(
    scanner: &Scanner<P>,
    scan_results_receiver: &mpsc::UnboundedReceiver<(ScanRange, Result<ScanResults, ScanError>)>,
    wallet: &W,
) -> bool
where
    P: consensus::Parameters + Sync + Send + 'static,
    W: SyncWallet,
{
    scanner.worker_poolsize() == 0
        && scan_results_receiver.is_empty()
        && wallet.get_sync_state().unwrap().scan_complete()
}

/// Scan post-processing
async fn process_scan_results<P, W>(
    consensus_parameters: &P,
    wallet: &mut W,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    scan_range: ScanRange,
    scan_results: Result<ScanResults, ScanError>,
    scanner_state: &mut ScannerState,
) -> Result<(), SyncError>
where
    P: consensus::Parameters,
    W: SyncWallet + SyncBlocks + SyncTransactions + SyncNullifiers + SyncOutPoints + SyncShardTrees,
{
    match scan_results {
        Ok(results) => {
            if scan_range.priority() == ScanPriority::Verify {
                scanner_state.verify();
            }

            update_wallet_data(wallet, results).unwrap();
            update_shielded_spends(consensus_parameters, wallet, fetch_request_sender, ufvks)
                .await
                .unwrap();
            update_transparent_spends(wallet).unwrap();
            remove_irrelevant_data(wallet, &scan_range).unwrap();
            state::set_scan_priority(
                wallet.get_sync_state_mut().unwrap(),
                scan_range.block_range(),
                ScanPriority::Scanned,
            )
            .unwrap();
            tracing::info!("Scan results processed.");
        }
        Err(ScanError::ContinuityError(ContinuityError::HashDiscontinuity { height, .. })) => {
            tracing::info!("Re-org detected.");
            if height == scan_range.block_range().start {
                // error handling in case of re-org where first block prev_hash in scan range does not match previous wallet block hash
                let sync_state = wallet.get_sync_state_mut().unwrap();
                state::set_scan_priority(
                    sync_state,
                    scan_range.block_range(),
                    scan_range.priority(),
                )
                .unwrap(); // reset scan range to initial priority in wallet sync state
                let scan_range_to_verify = state::set_verify_scan_range(
                    sync_state,
                    height - 1,
                    state::VerifyEnd::VerifyHighest,
                );
                truncate_wallet_data(wallet, scan_range_to_verify.block_range().start - 1).unwrap();
            } else {
                scan_results?;
            }
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

async fn process_mempool_stream_response<W>(
    consensus_parameters: &impl consensus::Parameters,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    wallet: &mut W,
    mempool_stream_response: Result<Option<RawTransaction>, tonic::Status>,
    mempool_stream: &mut tonic::Streaming<RawTransaction>,
) where
    W: SyncWallet + SyncBlocks + SyncTransactions + SyncNullifiers + SyncOutPoints,
{
    match mempool_stream_response.unwrap() {
        Some(raw_transaction) => {
            let block_height =
                BlockHeight::from_u32(u32::try_from(raw_transaction.height).unwrap());
            let transaction = zcash_primitives::transaction::Transaction::read(
                &raw_transaction.data[..],
                consensus::BranchId::for_height(consensus_parameters, block_height),
            )
            .unwrap();

            tracing::info!(
                "mempool received txid {} at height {}",
                transaction.txid(),
                block_height
            );

            if let Some(tx) = wallet
                .get_wallet_transactions()
                .unwrap()
                .get(&transaction.txid())
            {
                if tx.confirmation_status().is_confirmed() {
                    return;
                }
            }

            let confirmation_status = ConfirmationStatus::Mempool(block_height);
            let mut nullifiers = NullifierMap::new();
            let mut outpoints = OutPointMap::new();
            let transparent_addresses: HashMap<String, TransparentAddressId> = wallet
                .get_transparent_addresses()
                .unwrap()
                .iter()
                .map(|(id, address)| (address.clone(), *id))
                .collect();
            let mempool_transaction = scan_transaction(
                consensus_parameters,
                ufvks,
                transaction,
                confirmation_status,
                &DecryptedNoteData::new(),
                &mut nullifiers,
                &mut outpoints,
                &transparent_addresses,
            )
            .unwrap();

            // only add to wallet if tx is relavent
            if mempool_transaction.transparent_coins().is_empty()
                && mempool_transaction.sapling_notes().is_empty()
                && mempool_transaction.orchard_notes().is_empty()
                && mempool_transaction.outgoing_orchard_notes().is_empty()
                && mempool_transaction.outgoing_sapling_notes().is_empty()
            {
                return;
            }

            // TODO: there is an edge case where txs sent to the mempool that do not return change or create outgoing notes
            // will not show up
            wallet
                .insert_wallet_transaction(mempool_transaction)
                .unwrap();
            wallet.append_nullifiers(nullifiers).unwrap();
            wallet.append_outpoints(outpoints).unwrap();
            update_shielded_spends(
                consensus_parameters,
                wallet,
                fetch_request_sender.clone(),
                ufvks,
            )
            .await
            .unwrap();
            update_transparent_spends(wallet).unwrap();

            // TODO: consider logic for pending spent being set back to None when txs are evicted / never make it on chain
            // similar logic to truncate
        }
        None => {
            // A block was mined, setup a new mempool stream until the next block is mined.
            *mempool_stream = client::get_mempool_transaction_stream(fetch_request_sender.clone())
                .await
                .unwrap();
        }
    }
}

/// Removes all wallet data above the given `truncate_height`.
fn truncate_wallet_data<W>(wallet: &mut W, truncate_height: BlockHeight) -> Result<(), ()>
where
    W: SyncBlocks + SyncTransactions + SyncNullifiers + SyncShardTrees,
{
    wallet.truncate_wallet_blocks(truncate_height).unwrap();
    wallet
        .truncate_wallet_transactions(truncate_height)
        .unwrap();
    wallet.truncate_nullifiers(truncate_height).unwrap();
    wallet.truncate_shard_trees(truncate_height).unwrap();

    Ok(())
}

fn update_wallet_data<W>(wallet: &mut W, scan_results: ScanResults) -> Result<(), ()>
where
    W: SyncBlocks + SyncTransactions + SyncNullifiers + SyncOutPoints + SyncShardTrees,
{
    let ScanResults {
        nullifiers,
        wallet_blocks,
        wallet_transactions,
        shard_tree_data,
        outpoints,
    } = scan_results;

    wallet.append_wallet_blocks(wallet_blocks).unwrap();
    wallet
        .extend_wallet_transactions(wallet_transactions)
        .unwrap();
    wallet.append_nullifiers(nullifiers).unwrap();
    wallet.append_outpoints(outpoints).unwrap();
    // TODO: pararellise shard tree, this is currently the bottleneck on sync
    wallet.update_shard_trees(shard_tree_data).unwrap();
    // TODO: add trait to save wallet data to persistance for in-memory wallets

    Ok(())
}

/// Locates any nullifiers of notes in the wallet's transactions which match a nullifier in the wallet's nullifier map.
/// If a spend is detected, the nullifier is removed from the nullifier map and added to the map of spend locators.
/// The spend locators are then used to fetch and scan the transactions with detected spends.
/// Finally, all notes that were detected as spent are updated with the located spending transaction.
async fn update_shielded_spends<P, W>(
    consensus_parameters: &P,
    wallet: &mut W,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
) -> Result<(), ()>
where
    P: consensus::Parameters,
    W: SyncBlocks + SyncTransactions + SyncNullifiers,
{
    let (sapling_nullifiers, orchard_nullifiers) =
        collect_derived_nullifiers(wallet.get_wallet_transactions().unwrap()).unwrap();

    let (sapling_spend_locators, orchard_spend_locators) = detect_shielded_spends(
        wallet.get_nullifiers_mut().unwrap(),
        sapling_nullifiers,
        orchard_nullifiers,
    )
    .unwrap();

    scan_located_transactions(
        fetch_request_sender,
        consensus_parameters,
        wallet,
        ufvks,
        sapling_spend_locators
            .values()
            .chain(orchard_spend_locators.values())
            .cloned(),
    )
    .await
    .unwrap();

    update_spent_notes(
        wallet.get_wallet_transactions_mut().unwrap(),
        sapling_spend_locators,
        orchard_spend_locators,
    )
    .unwrap();

    Ok(())
}

/// Collects the derived nullifiers from each note in the wallet
fn collect_derived_nullifiers(
    wallet_transactions: &HashMap<TxId, WalletTransaction>,
) -> Result<
    (
        Vec<sapling_crypto::Nullifier>,
        Vec<orchard::note::Nullifier>,
    ),
    (),
> {
    let sapling_nullifiers = wallet_transactions
        .values()
        .flat_map(|tx| tx.sapling_notes())
        .flat_map(|note| note.nullifier())
        .collect::<Vec<_>>();
    let orchard_nullifiers = wallet_transactions
        .values()
        .flat_map(|tx| tx.orchard_notes())
        .flat_map(|note| note.nullifier())
        .collect::<Vec<_>>();

    Ok((sapling_nullifiers, orchard_nullifiers))
}

fn detect_shielded_spends(
    nullifier_map: &mut NullifierMap,
    sapling_nullifiers: Vec<sapling_crypto::Nullifier>,
    orchard_nullifiers: Vec<orchard::note::Nullifier>,
) -> Result<
    (
        BTreeMap<sapling_crypto::Nullifier, Locator>,
        BTreeMap<orchard::note::Nullifier, Locator>,
    ),
    (),
> {
    let sapling_spend_locators = sapling_nullifiers
        .iter()
        .flat_map(|nf| nullifier_map.sapling_mut().remove_entry(nf))
        .collect();
    let orchard_spend_locators = orchard_nullifiers
        .iter()
        .flat_map(|nf| nullifier_map.orchard_mut().remove_entry(nf))
        .collect();

    Ok((sapling_spend_locators, orchard_spend_locators))
}

// in the edge case where a spending transaction received no change, scan the transactions that evaded trial decryption
///
async fn scan_located_transactions<L, P, W>(
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    consensus_parameters: &P,
    wallet: &mut W,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    locators: L,
) -> Result<(), ()>
where
    L: Iterator<Item = Locator>,
    P: consensus::Parameters,
    W: SyncBlocks + SyncTransactions + SyncNullifiers,
{
    let wallet_transactions = wallet.get_wallet_transactions().unwrap();
    let wallet_txids = wallet_transactions.keys().copied().collect::<HashSet<_>>();
    let mut spending_txids = HashSet::new();
    let mut wallet_blocks = BTreeMap::new();
    for (block_height, txid) in locators {
        // skip if transaction already exists in the wallet
        if wallet_txids.contains(&txid) {
            continue;
        }

        spending_txids.insert(txid);
        wallet_blocks.insert(block_height, wallet.get_wallet_block(block_height).unwrap());
    }

    let mut outpoint_map = OutPointMap::new(); // dummy outpoint map
    let spending_transactions = scan_transactions(
        fetch_request_sender,
        consensus_parameters,
        ufvks,
        spending_txids,
        DecryptedNoteData::new(),
        &wallet_blocks,
        &mut outpoint_map,
        HashMap::new(), // no need to scan transparent bundles as all relevant txs will not be evaded during scanning
    )
    .await
    .unwrap();
    wallet
        .extend_wallet_transactions(spending_transactions)
        .unwrap();

    Ok(())
}

/// Update the spending transaction for all notes where the derived nullifier matches the nullifier in the spend locator map.
/// The items in the spend locator map are taken directly from the nullifier map during spend detection.
fn update_spent_notes(
    wallet_transactions: &mut HashMap<TxId, WalletTransaction>,
    sapling_spend_locators: BTreeMap<sapling_crypto::Nullifier, Locator>,
    orchard_spend_locators: BTreeMap<orchard::note::Nullifier, Locator>,
) -> Result<(), ()> {
    wallet_transactions
        .values_mut()
        .flat_map(|tx| tx.sapling_notes_mut())
        .for_each(|note| {
            if let Some((_, txid)) = note
                .nullifier()
                .and_then(|nf| sapling_spend_locators.get(&nf))
            {
                note.set_spending_transaction(Some(*txid));
            }
        });
    wallet_transactions
        .values_mut()
        .flat_map(|tx| tx.orchard_notes_mut())
        .for_each(|note| {
            if let Some((_, txid)) = note
                .nullifier()
                .and_then(|nf| orchard_spend_locators.get(&nf))
            {
                note.set_spending_transaction(Some(*txid));
            }
        });

    Ok(())
}

fn update_transparent_spends<W>(wallet: &mut W) -> Result<(), ()>
where
    W: SyncBlocks + SyncTransactions + SyncOutPoints,
{
    // locate spends
    let wallet_transactions = wallet.get_wallet_transactions().unwrap();
    let transparent_output_ids = wallet_transactions
        .values()
        .flat_map(|tx| tx.transparent_coins())
        .map(|coin| coin.output_id())
        .collect::<Vec<_>>();
    let outpoint_map = wallet.get_outpoints_mut().unwrap();
    let transparent_spend_locators: BTreeMap<OutputId, Locator> = transparent_output_ids
        .iter()
        .flat_map(|output_id| outpoint_map.inner_mut().remove_entry(output_id))
        .collect();

    // add spending transaction for all spent coins
    let wallet_transactions = wallet.get_wallet_transactions_mut().unwrap();
    wallet_transactions
        .values_mut()
        .flat_map(|tx| tx.transparent_coins_mut())
        .for_each(|coin| {
            if let Some((_, txid)) = transparent_spend_locators.get(&coin.output_id()) {
                coin.set_spending_transaction(Some(*txid));
            }
        });

    Ok(())
}

// TODO: replace this function with a filter on the data added to wallet
fn remove_irrelevant_data<W>(wallet: &mut W, scan_range: &ScanRange) -> Result<(), ()>
where
    W: SyncWallet + SyncBlocks + SyncNullifiers + SyncTransactions,
{
    if scan_range.priority() != ScanPriority::Historic {
        return Ok(());
    }

    let wallet_height = wallet
        .get_sync_state()
        .unwrap()
        .scan_ranges()
        .last()
        .expect("wallet should always have scan ranges after sync has started")
        .block_range()
        .end;

    let wallet_transaction_heights = wallet
        .get_wallet_transactions()
        .unwrap()
        .values()
        .filter_map(|tx| tx.confirmation_status().get_confirmed_height())
        .collect::<Vec<_>>();
    wallet.get_wallet_blocks_mut().unwrap().retain(|height, _| {
        *height >= scan_range.block_range().end - 1
            || *height >= wallet_height - 100
            || wallet_transaction_heights.contains(height)
    });
    wallet
        .get_nullifiers_mut()
        .unwrap()
        .sapling_mut()
        .retain(|_, (height, _)| *height >= scan_range.block_range().end);
    wallet
        .get_nullifiers_mut()
        .unwrap()
        .orchard_mut()
        .retain(|_, (height, _)| *height >= scan_range.block_range().end);

    Ok(())
}
