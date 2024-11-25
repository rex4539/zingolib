//! Entrypoint for sync engine

use std::cmp;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::{Add, Range};
use std::time::Duration;

use crate::client::fetch::fetch;
use crate::client::{self, FetchRequest};
use crate::error::SyncError;
use crate::keys::{AddressIndex, TransparentAddressId, TransparentScope};
use crate::primitives::SyncState;
use crate::scan::error::{ContinuityError, ScanError};
use crate::scan::task::{Scanner, ScannerState};
use crate::scan::transactions::scan_transactions;
use crate::scan::{DecryptedNoteData, ScanResults};
use crate::traits::{SyncBlocks, SyncNullifiers, SyncShardTrees, SyncTransactions, SyncWallet};

use zcash_address::{ToAddress, ZcashAddress};
use zcash_client_backend::proto::service::GetAddressUtxosReply;
use zcash_client_backend::{
    data_api::scanning::{ScanPriority, ScanRange},
    proto::service::compact_tx_streamer_client::CompactTxStreamerClient,
};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::{self, BlockHeight, NetworkUpgrade};

use tokio::sync::mpsc;
use zcash_primitives::legacy::keys::{AccountPubKey, IncomingViewingKey, NonHardenedChildIndex};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::AccountId;

// TODO: create sub modules for sync module to organise code

// TODO; replace fixed batches with orchard shard ranges (block ranges containing all note commitments to an orchard shard or fragment of a shard)
const BATCH_SIZE: u32 = 1_000;
const VERIFY_BLOCK_RANGE_SIZE: u32 = 10;
const ADDRESS_GAP_LIMIT: u32 = 20;

/// Syncs a wallet to the latest state of the blockchain
pub async fn sync<P, W>(
    client: CompactTxStreamerClient<zingo_netutils::UnderlyingService>, // TODO: change underlying service for generic
    consensus_parameters: &P,
    wallet: &mut W,
) -> Result<(), SyncError>
where
    P: consensus::Parameters + Sync + Send + 'static,
    W: SyncWallet + SyncBlocks + SyncTransactions + SyncNullifiers + SyncShardTrees,
{
    let previous_sync_wallet_height =
        if let Some(highest_range) = wallet.get_sync_state().unwrap().scan_ranges().last() {
            highest_range.block_range().end - 1
        } else {
            wallet.get_birthday().unwrap() - 1
        };
    let ufvks = wallet.get_unified_full_viewing_keys().unwrap();

    tracing::info!("Syncing wallet...");

    // create channel for sending fetch requests and launch fetcher task
    let (fetch_request_sender, fetch_request_receiver) = mpsc::unbounded_channel();
    let fetcher_handle = tokio::spawn(fetch(
        fetch_request_receiver,
        client,
        consensus_parameters.clone(),
    ));

    discover_transparent_output_locators(
        wallet,
        fetch_request_sender.clone(),
        previous_sync_wallet_height,
    )
    .await;
    discover_transparent_addresses(
        wallet,
        consensus_parameters,
        fetch_request_sender.clone(),
        &ufvks,
        previous_sync_wallet_height,
    )
    .await;

    update_scan_ranges(
        fetch_request_sender.clone(),
        consensus_parameters,
        wallet.get_birthday().unwrap(),
        wallet.get_sync_state_mut().unwrap(),
    )
    .await
    .unwrap();

    // create channel for receiving scan results and launch scanner
    let (scan_results_sender, mut scan_results_receiver) = mpsc::unbounded_channel();
    let mut scanner = Scanner::new(
        scan_results_sender,
        fetch_request_sender.clone(),
        consensus_parameters.clone(),
        ufvks.clone(),
    );
    scanner.spawn_workers();

    let mut interval = tokio::time::interval(Duration::from_millis(30));
    loop {
        tokio::select! {
            Some((scan_range, scan_results)) = scan_results_receiver.recv() => {
                process_scan_results(
                    wallet,
                    fetch_request_sender.clone(),
                    consensus_parameters,
                    &ufvks,
                    scan_range,
                    scan_results,
                    scanner.state_mut(),
                )
                .await
                .unwrap();
            }

            _ = interval.tick() => {
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

/// Discovers the location of any transparent outputs associated with addresses known to the wallet above the wallet
/// height when the wallet was previously synced.
async fn discover_transparent_output_locators<W>(
    wallet: &mut W,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    previous_sync_wallet_height: BlockHeight,
) where
    W: SyncWallet,
{
    let wallet_addresses: Vec<String> = wallet
        .get_transparent_addresses()
        .unwrap()
        .iter()
        .map(|(_, address)| address.clone())
        .collect();

    if wallet_addresses.is_empty() {
        return;
    }

    // get the transparent output metadata from the server.
    let transparent_output_metadata = client::get_transparent_output_metadata(
        fetch_request_sender.clone(),
        wallet_addresses,
        previous_sync_wallet_height + 1,
    )
    .await
    .unwrap();

    // collect the transparent output locators
    let transparent_output_locators: Vec<(BlockHeight, TxId)> = transparent_output_metadata
        .iter()
        .map(|metadata| {
            (
                BlockHeight::from_u32(u32::try_from(metadata.height).unwrap()),
                TxId::from_bytes(<[u8; 32]>::try_from(&metadata.txid[..]).unwrap()),
            )
        })
        .collect();

    update_transparent_output_locators(wallet, transparent_output_locators);
}

/// Updates the wallet with any previously unknown transparent addresses that are now in use.
/// Also updates the transparent output locators for any newly added addresses.
async fn discover_transparent_addresses<P, W>(
    wallet: &mut W,
    consensus_parameters: &P,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    previous_sync_wallet_height: BlockHeight,
) where
    P: consensus::Parameters,
    W: SyncWallet,
{
    let mut transparent_output_locators: Vec<(BlockHeight, TxId)> = Vec::new();
    let mut wallet_addresses: Vec<(TransparentAddressId, String)> = wallet
        .get_transparent_addresses()
        .unwrap()
        .iter()
        .map(|(id, address)| (id.clone(), address.clone()))
        .collect();

    // derive additional addresses until all scopes in all accounts satisfy the address gap limit.
    loop {
        // derive addresses for all accounts and scopes together to minimise calls to the server.
        let gap_limit_addresses =
            derive_gap_limit_addresses(consensus_parameters, ufvks, &wallet_addresses);
        // tracing::info!("gap limit:\n{:#?}", &gap_limit_addresses);

        // get the transparent output metadata from the server.
        let transparent_output_metadata = client::get_transparent_output_metadata(
            fetch_request_sender.clone(),
            gap_limit_addresses
                .iter()
                .map(|(_, address)| address.clone())
                .collect(),
            previous_sync_wallet_height + 1,
        )
        .await
        .unwrap();
        // tracing::info!("outputs:\n{:#?}", &transparent_output_metadata);

        if transparent_output_metadata.is_empty() {
            // gap limit satisfied for all scopes of all accounts.
            break;
        }

        // collect the transparent output locators
        transparent_output_locators.append(
            transparent_output_metadata
                .iter()
                .map(|metadata| {
                    (
                        BlockHeight::from_u32(u32::try_from(metadata.height).unwrap()),
                        TxId::from_bytes(<[u8; 32]>::try_from(&metadata.txid[..]).unwrap()),
                    )
                })
                .collect::<Vec<_>>()
                .as_mut(),
        );

        // determine if any of these newly derived addresses are used
        add_new_addresses(
            ufvks,
            &mut wallet_addresses,
            &gap_limit_addresses,
            &transparent_output_metadata,
        );
        // tracing::info!("wallet:\n{:#?}", &wallet_addresses);
    }

    update_transparent_output_locators(wallet, transparent_output_locators);
    update_transparent_addresses(wallet, wallet_addresses);
}

fn update_transparent_output_locators<W>(
    wallet: &mut W,
    mut transparent_output_locators: Vec<(BlockHeight, TxId)>,
) where
    W: SyncWallet,
{
    let sync_state = wallet.get_sync_state_mut().unwrap();
    sync_state
        .transparent_output_locators_mut()
        .append(&mut transparent_output_locators);
    sync_state
        .transparent_output_locators_mut()
        .sort_unstable_by_key(|(height, _)| *height);
}

fn update_transparent_addresses<W>(
    wallet: &mut W,
    transparent_addresses: Vec<(TransparentAddressId, String)>,
) where
    W: SyncWallet,
{
    let wallet_addresses = wallet.get_transparent_addresses_mut().unwrap();
    for (id, address) in transparent_addresses {
        wallet_addresses.insert(id, address);
    }
}

/// For each scope in each account, extend `wallet addresses` by all `gap_limit_addresses` up to the highest index
/// address contained in `transparent_output_metadata`.
fn add_new_addresses(
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    wallet_addresses: &mut Vec<(TransparentAddressId, String)>,
    gap_limit_addresses: &[(TransparentAddressId, String)],
    transparent_output_metadata: &[GetAddressUtxosReply],
) {
    let transparent_metadata_addresses: Vec<&str> = transparent_output_metadata
        .iter()
        .map(|metadata| metadata.address.as_str())
        .collect();

    for (account_id, ufvk) in ufvks {
        if ufvk.transparent().is_some() {
            for scope in [
                TransparentScope::External,
                TransparentScope::Internal,
                TransparentScope::Refund,
            ] {
                let mut gap_limit_addresses_by_scope = gap_limit_addresses
                    .iter()
                    .filter(|(id, _)| id.account_id() == *account_id && id.scope() == scope)
                    .cloned()
                    .collect::<Vec<_>>();

                let highest_new_address_index: Option<usize> = if let Some((id, _)) =
                    gap_limit_addresses_by_scope
                        .iter()
                        .rev()
                        .find(|(_, address)| {
                            transparent_metadata_addresses.contains(&address.as_str())
                        }) {
                    Some(id.address_index() as usize)
                } else {
                    None
                };

                if let Some(address_index) = highest_new_address_index {
                    wallet_addresses.extend(
                        gap_limit_addresses_by_scope
                            .drain(..address_index + 1)
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
    }
}

/// Derives new addresses for all accounts and scopes.
///
/// For each scope in each account, find the highest address known to the wallet and derive [`self::ADDRESS_GAP_LIMIT`]
/// more addresses from the lowest unknown address index.
fn derive_gap_limit_addresses<P>(
    consensus_parameters: &P,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    wallet_addresses: &[(TransparentAddressId, String)],
) -> Vec<(TransparentAddressId, String)>
where
    P: consensus::Parameters,
{
    let mut gap_limit_addresses: Vec<(TransparentAddressId, String)> = Vec::new();

    for (account_id, ufvk) in ufvks {
        if let Some(account_pubkey) = ufvk.transparent() {
            for scope in [
                TransparentScope::External,
                TransparentScope::Internal,
                TransparentScope::Refund,
            ] {
                let lowest_unknown_index = if let Some(id) = wallet_addresses
                    .iter()
                    .map(|(id, _)| id)
                    .filter(|id| id.account_id() == *account_id && id.scope() == scope)
                    .rev()
                    .next()
                {
                    id.address_index() + 1
                } else {
                    0
                };

                gap_limit_addresses.extend(derive_transparent_addresses_by_scope(
                    consensus_parameters,
                    account_pubkey,
                    account_id.clone(),
                    scope,
                    lowest_unknown_index,
                ));
            }
        }
    }

    gap_limit_addresses
}

fn derive_transparent_addresses_by_scope<P>(
    consensus_parameters: &P,
    account_pubkey: &AccountPubKey,
    account_id: AccountId,
    scope: TransparentScope,
    lowest_unknown_index: AddressIndex,
) -> Vec<(TransparentAddressId, String)>
where
    P: consensus::Parameters,
{
    let address_index_range: Range<AddressIndex> = Range {
        start: lowest_unknown_index,
        end: lowest_unknown_index + ADDRESS_GAP_LIMIT,
    };

    match scope {
        TransparentScope::External => derive_transparent_addresses(
            consensus_parameters,
            account_id,
            scope,
            address_index_range.clone(),
            |address_index| {
                account_pubkey
                    .derive_external_ivk()
                    .unwrap()
                    .derive_address(
                        NonHardenedChildIndex::from_index(address_index)
                            .expect("all non-hardened address indexes in use!"),
                    )
                    .unwrap()
            },
        ),
        TransparentScope::Internal => derive_transparent_addresses(
            consensus_parameters,
            account_id,
            scope,
            address_index_range.clone(),
            |address_index| {
                account_pubkey
                    .derive_internal_ivk()
                    .unwrap()
                    .derive_address(
                        NonHardenedChildIndex::from_index(address_index)
                            .expect("all non-hardened address indexes in use!"),
                    )
                    .unwrap()
            },
        ),
        TransparentScope::Refund => derive_transparent_addresses(
            consensus_parameters,
            account_id,
            scope,
            address_index_range.clone(),
            |address_index| {
                account_pubkey
                    .derive_ephemeral_ivk()
                    .unwrap()
                    .derive_ephemeral_address(
                        NonHardenedChildIndex::from_index(address_index)
                            .expect("all non-hardened address indexes in use!"),
                    )
                    .unwrap()
            },
        ),
    }
}

fn derive_transparent_addresses<F, P>(
    consensus_parameters: &P,
    account_id: AccountId,
    scope: TransparentScope,
    address_index_range: Range<AddressIndex>,
    derive_address: F,
) -> Vec<(TransparentAddressId, String)>
where
    F: Fn(u32) -> TransparentAddress,
    P: consensus::Parameters,
{
    address_index_range
        .into_iter()
        .map(|address_index| {
            let key_id = TransparentAddressId::from_parts(account_id, scope, address_index.into());
            let address = derive_address(address_index);
            let zcash_address = match address {
                TransparentAddress::PublicKeyHash(data) => {
                    ZcashAddress::from_transparent_p2pkh(consensus_parameters.network_type(), data)
                }
                TransparentAddress::ScriptHash(data) => {
                    ZcashAddress::from_transparent_p2sh(consensus_parameters.network_type(), data)
                }
            };
            (key_id, zcash_address.to_string())
        })
        .collect()
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

/// Update scan ranges for scanning
async fn update_scan_ranges<P>(
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    consensus_parameters: &P,
    wallet_birthday: BlockHeight,
    sync_state: &mut SyncState,
) -> Result<(), ()>
where
    P: consensus::Parameters,
{
    let chain_height = client::get_chain_height(fetch_request_sender)
        .await
        .unwrap();
    create_scan_range(
        chain_height,
        consensus_parameters,
        wallet_birthday,
        sync_state,
    )
    .await?;
    reset_scan_ranges(sync_state)?;
    set_verification_scan_range(sync_state)?;

    // TODO: add logic to merge scan ranges
    // TODO: set chain tip range
    // TODO: set open adjacent range

    Ok(())
}

/// Create scan range between the last known chain height (wallet height) and the chain height from the server
async fn create_scan_range<P>(
    chain_height: BlockHeight,
    consensus_parameters: &P,
    wallet_birthday: BlockHeight,
    sync_state: &mut SyncState,
) -> Result<(), ()>
where
    P: consensus::Parameters,
{
    let scan_ranges = sync_state.scan_ranges_mut();

    let wallet_height = if scan_ranges.is_empty() {
        let sapling_activation_height = consensus_parameters
            .activation_height(NetworkUpgrade::Sapling)
            .expect("sapling activation height should always return Some");

        match wallet_birthday.cmp(&sapling_activation_height) {
            cmp::Ordering::Greater | cmp::Ordering::Equal => wallet_birthday,
            cmp::Ordering::Less => sapling_activation_height,
        }
    } else {
        scan_ranges
            .last()
            .expect("Vec should not be empty")
            .block_range()
            .end
    };

    if wallet_height > chain_height {
        // TODO:  truncate wallet to server height in case of reorg
        panic!("wallet is ahead of server!")
    }

    let new_scan_range = ScanRange::from_parts(
        Range {
            start: wallet_height,
            end: chain_height + 1,
        },
        ScanPriority::Historic,
    );
    scan_ranges.push(new_scan_range);

    if scan_ranges.is_empty() {
        panic!("scan ranges should never be empty after updating");
    }

    Ok(())
}

fn reset_scan_ranges(sync_state: &mut SyncState) -> Result<(), ()> {
    let scan_ranges = sync_state.scan_ranges_mut();
    let stale_verify_scan_ranges = scan_ranges
        .iter()
        .filter(|range| range.priority() == ScanPriority::Verify)
        .cloned()
        .collect::<Vec<_>>();
    let previously_scanning_scan_ranges = scan_ranges
        .iter()
        .filter(|range| range.priority() == ScanPriority::Ignored)
        .cloned()
        .collect::<Vec<_>>();
    for scan_range in stale_verify_scan_ranges {
        set_scan_priority(
            sync_state,
            scan_range.block_range(),
            ScanPriority::OpenAdjacent,
        )
        .unwrap();
    }
    // a range that was previously scanning when sync was last interupted should be set to `ChainTip` which is the
    // highest priority that is not `Verify`.
    for scan_range in previously_scanning_scan_ranges {
        set_scan_priority(sync_state, scan_range.block_range(), ScanPriority::ChainTip).unwrap();
    }

    Ok(())
}

fn set_verification_scan_range(sync_state: &mut SyncState) -> Result<(), ()> {
    let scan_ranges = sync_state.scan_ranges_mut();
    if let Some((index, lowest_unscanned_range)) =
        scan_ranges.iter().enumerate().find(|(_, scan_range)| {
            scan_range.priority() != ScanPriority::Ignored
                && scan_range.priority() != ScanPriority::Scanned
        })
    {
        let block_range_to_verify = Range {
            start: lowest_unscanned_range.block_range().start,
            end: lowest_unscanned_range
                .block_range()
                .start
                .add(VERIFY_BLOCK_RANGE_SIZE),
        };
        let split_ranges = split_out_scan_range(
            lowest_unscanned_range,
            block_range_to_verify,
            ScanPriority::Verify,
        );

        sync_state
            .scan_ranges_mut()
            .splice(index..=index, split_ranges);
    }

    Ok(())
}

/// Selects and prepares the next scan range for scanning.
/// Sets the range for scanning to `Ignored` priority in the wallet `sync_state` but returns the scan range with its initial priority.
/// Returns `None` if there are no more ranges to scan.
pub(crate) fn select_scan_range(sync_state: &mut SyncState) -> Option<ScanRange> {
    let scan_ranges = sync_state.scan_ranges_mut();

    let mut scan_ranges_priority_sorted: Vec<&ScanRange> = scan_ranges.iter().collect();
    scan_ranges_priority_sorted.sort_by(|a, b| b.block_range().start.cmp(&a.block_range().start));
    scan_ranges_priority_sorted.sort_by_key(|scan_range| scan_range.priority());
    let highest_priority_scan_range = scan_ranges_priority_sorted
        .pop()
        .expect("scan ranges should be non-empty after setup")
        .clone();
    if highest_priority_scan_range.priority() == ScanPriority::Scanned
        || highest_priority_scan_range.priority() == ScanPriority::Ignored
    {
        return None;
    }

    let (index, selected_scan_range) = scan_ranges
        .iter_mut()
        .enumerate()
        .find(|(_, scan_range)| {
            scan_range.block_range() == highest_priority_scan_range.block_range()
        })
        .expect("scan range should exist");

    let batch_block_range = Range {
        start: selected_scan_range.block_range().start,
        end: selected_scan_range.block_range().start + BATCH_SIZE,
    };
    let split_ranges = split_out_scan_range(
        selected_scan_range,
        batch_block_range,
        ScanPriority::Ignored,
    );

    let trimmed_block_range = split_ranges
        .first()
        .expect("vec should always be non-empty")
        .block_range()
        .clone();

    scan_ranges.splice(index..=index, split_ranges);

    // TODO: when this library has its own version of ScanRange this can be simpified and more readable
    Some(ScanRange::from_parts(
        trimmed_block_range,
        highest_priority_scan_range.priority(),
    ))
}

/// Scan post-processing
async fn process_scan_results<P, W>(
    wallet: &mut W,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    consensus_parameters: &P,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    scan_range: ScanRange,
    scan_results: Result<ScanResults, ScanError>,
    scanner_state: &mut ScannerState,
) -> Result<(), SyncError>
where
    P: consensus::Parameters,
    W: SyncWallet + SyncBlocks + SyncTransactions + SyncNullifiers + SyncShardTrees,
{
    match scan_results {
        Ok(results) => {
            if scan_range.priority() == ScanPriority::Verify {
                scanner_state.verify();
            }

            update_wallet_data(wallet, results).unwrap();
            link_nullifiers(wallet, fetch_request_sender, consensus_parameters, ufvks)
                .await
                .unwrap();
            remove_irrelevant_data(wallet, &scan_range).unwrap();
            set_scan_priority(
                wallet.get_sync_state_mut().unwrap(),
                scan_range.block_range(),
                ScanPriority::Scanned,
            )
            .unwrap();
            // TODO: also combine adjacent scanned ranges together
            tracing::info!("Scan results processed.");
        }
        Err(ScanError::ContinuityError(ContinuityError::HashDiscontinuity { height, .. })) => {
            tracing::info!("Re-org detected.");
            if height == scan_range.block_range().start {
                // error handling in case of re-org where first block prev_hash in scan range does not match previous wallet block hash
                let sync_state = wallet.get_sync_state_mut().unwrap();
                set_scan_priority(sync_state, scan_range.block_range(), scan_range.priority())
                    .unwrap(); // reset scan range to initial priority in wallet sync state
                let scan_range_to_verify = verify_scan_range_tip(sync_state, height - 1);
                truncate_wallet_data(wallet, scan_range_to_verify.block_range().start - 1).unwrap();
            } else {
                scan_results?;
            }
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
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
    W: SyncBlocks + SyncTransactions + SyncNullifiers + SyncShardTrees,
{
    let ScanResults {
        nullifiers,
        wallet_blocks,
        wallet_transactions,
        shard_tree_data,
    } = scan_results;

    wallet.append_wallet_blocks(wallet_blocks).unwrap();
    wallet
        .extend_wallet_transactions(wallet_transactions)
        .unwrap();
    wallet.append_nullifiers(nullifiers).unwrap();
    // TODO: pararellise shard tree, this is currently the bottleneck on sync
    wallet.update_shard_trees(shard_tree_data).unwrap();
    // TODO: add trait to save wallet data to persistance for in-memory wallets

    Ok(())
}

async fn link_nullifiers<P, W>(
    wallet: &mut W,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    parameters: &P,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
) -> Result<(), ()>
where
    P: consensus::Parameters,
    W: SyncBlocks + SyncTransactions + SyncNullifiers,
{
    // locate spends
    let wallet_transactions = wallet.get_wallet_transactions().unwrap();
    let wallet_txids = wallet_transactions.keys().copied().collect::<HashSet<_>>();
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

    let nullifier_map = wallet.get_nullifiers_mut().unwrap();
    let sapling_spend_locators: BTreeMap<sapling_crypto::Nullifier, (BlockHeight, TxId)> =
        sapling_nullifiers
            .iter()
            .flat_map(|nf| nullifier_map.sapling_mut().remove_entry(nf))
            .collect();
    let orchard_spend_locators: BTreeMap<orchard::note::Nullifier, (BlockHeight, TxId)> =
        orchard_nullifiers
            .iter()
            .flat_map(|nf| nullifier_map.orchard_mut().remove_entry(nf))
            .collect();

    // in the edge case where a spending transaction received no change, scan the transactions that evaded trial decryption
    let mut spending_txids = HashSet::new();
    let mut wallet_blocks = BTreeMap::new();
    for (block_height, txid) in sapling_spend_locators
        .values()
        .chain(orchard_spend_locators.values())
    {
        // skip if transaction already exists in the wallet
        if wallet_txids.contains(txid) {
            continue;
        }

        spending_txids.insert(*txid);
        wallet_blocks.insert(
            *block_height,
            wallet.get_wallet_block(*block_height).unwrap(),
        );
    }
    let spending_transactions = scan_transactions(
        fetch_request_sender,
        parameters,
        ufvks,
        spending_txids,
        DecryptedNoteData::new(),
        &wallet_blocks,
    )
    .await
    .unwrap();
    wallet
        .extend_wallet_transactions(spending_transactions)
        .unwrap();

    // add spending transaction for all spent notes
    let wallet_transactions = wallet.get_wallet_transactions_mut().unwrap();
    wallet_transactions
        .values_mut()
        .flat_map(|tx| tx.sapling_notes_mut())
        .filter(|note| note.spending_transaction().is_none())
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
        .filter(|note| note.spending_transaction().is_none())
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

/// Splits out the highest VERIFY_BLOCK_RANGE_SIZE blocks from the scan range containing the given `block height`
/// and sets it's priority to `Verify`.
/// Returns a clone of the scan range to be verified.
///
/// Panics if the scan range containing the given block height is not of priority `Scanned`
fn verify_scan_range_tip(sync_state: &mut SyncState, block_height: BlockHeight) -> ScanRange {
    let (index, scan_range) = sync_state
        .scan_ranges()
        .iter()
        .enumerate()
        .find(|(_, range)| range.block_range().contains(&block_height))
        .expect("scan range containing given block height should always exist!");

    if scan_range.priority() != ScanPriority::Scanned {
        panic!("scan range should always have scan priority `Scanned`!")
    }

    let block_range_to_verify = Range {
        start: scan_range.block_range().end - VERIFY_BLOCK_RANGE_SIZE,
        end: scan_range.block_range().end,
    };
    let split_ranges =
        split_out_scan_range(scan_range, block_range_to_verify, ScanPriority::Verify);

    let scan_range_to_verify = split_ranges
        .last()
        .expect("vec should always be non-empty")
        .clone();

    sync_state
        .scan_ranges_mut()
        .splice(index..=index, split_ranges);

    scan_range_to_verify
}

/// Splits out a scan range surrounding a given block height with the specified priority
#[allow(dead_code)]
fn update_scan_priority(
    sync_state: &mut SyncState,
    block_height: BlockHeight,
    scan_priority: ScanPriority,
) {
    let (index, scan_range) = sync_state
        .scan_ranges()
        .iter()
        .enumerate()
        .find(|(_, range)| range.block_range().contains(&block_height))
        .expect("scan range should always exist for mapped nullifiers");

    // Skip if the given block height is within a range that is scanned or being scanning
    if scan_range.priority() == ScanPriority::Scanned
        || scan_range.priority() == ScanPriority::Ignored
    {
        return;
    }

    let new_block_range = determine_block_range(block_height);
    let split_ranges = split_out_scan_range(scan_range, new_block_range, scan_priority);
    sync_state
        .scan_ranges_mut()
        .splice(index..=index, split_ranges);
}

/// Determines which range of blocks should be scanned for a given block height
fn determine_block_range(block_height: BlockHeight) -> Range<BlockHeight> {
    let start = block_height - (u32::from(block_height) % BATCH_SIZE); // TODO: will be replaced with first block of associated orchard shard
    let end = start + BATCH_SIZE; // TODO: will be replaced with last block of associated orchard shard
    Range { start, end }
}

/// Takes a scan range and splits it at `block_range.start` and `block_range.end`, returning a vec of scan ranges where
/// the scan range with the specified `block_range` has the given `scan_priority`.
///
/// If `block_range` goes beyond the bounds of `scan_range.block_range()` no splitting will occur at the upper and/or
/// lower bound but the priority will still be updated
///
/// Panics if no blocks in `block_range` are contained within `scan_range.block_range()`
fn split_out_scan_range(
    scan_range: &ScanRange,
    block_range: Range<BlockHeight>,
    scan_priority: ScanPriority,
) -> Vec<ScanRange> {
    let mut split_ranges = Vec::new();
    if let Some((lower_range, higher_range)) = scan_range.split_at(block_range.start) {
        split_ranges.push(lower_range);
        if let Some((middle_range, higher_range)) = higher_range.split_at(block_range.end) {
            // [scan_range] is split at the upper and lower bound of [block_range]
            split_ranges.push(ScanRange::from_parts(
                middle_range.block_range().clone(),
                scan_priority,
            ));
            split_ranges.push(higher_range);
        } else {
            // [scan_range] is split only at the lower bound of [block_range]
            split_ranges.push(ScanRange::from_parts(
                higher_range.block_range().clone(),
                scan_priority,
            ));
        }
    } else if let Some((lower_range, higher_range)) = scan_range.split_at(block_range.end) {
        // [scan_range] is split only at the upper bound of [block_range]
        split_ranges.push(ScanRange::from_parts(
            lower_range.block_range().clone(),
            scan_priority,
        ));
        split_ranges.push(higher_range);
    } else {
        // [scan_range] is not split as it is fully contained within [block_range]
        // only scan priority is updated
        assert!(scan_range.block_range().start >= block_range.start);
        assert!(scan_range.block_range().end <= block_range.end);

        split_ranges.push(ScanRange::from_parts(
            scan_range.block_range().clone(),
            scan_priority,
        ));
    };

    split_ranges
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
        .map(|tx| tx.block_height())
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

/// Sets the scan range in `sync_state` with `block_range` to the given `scan_priority`.
///
/// Panics if no scan range is found in `sync_state` with a block range of exactly `block_range`.
fn set_scan_priority(
    sync_state: &mut SyncState,
    block_range: &Range<BlockHeight>,
    scan_priority: ScanPriority,
) -> Result<(), ()> {
    let scan_ranges = sync_state.scan_ranges_mut();

    if let Some((index, range)) = scan_ranges
        .iter()
        .enumerate()
        .find(|(_, range)| range.block_range() == block_range)
    {
        scan_ranges[index] = ScanRange::from_parts(range.block_range().clone(), scan_priority);
    } else {
        panic!("scan range with block range {:?} not found!", block_range)
    }

    Ok(())
}
