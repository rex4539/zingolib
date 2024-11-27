//! Module for handling all connections to the server

use std::ops::Range;

use tokio::sync::{mpsc::UnboundedSender, oneshot};

use zcash_client_backend::{
    data_api::chain::ChainState,
    proto::{
        compact_formats::CompactBlock,
        service::{BlockId, GetAddressUtxosReply, TreeState},
    },
};
use zcash_primitives::{
    consensus::BlockHeight,
    transaction::{Transaction, TxId},
};

pub mod fetch;

/// Fetch requests are created and sent to the [`crate::client::fetch::fetch`] task when a connection to the server is required.
///
/// Each variant includes a [`tokio::sync::oneshot::Sender`] for returning the fetched data to the requester.
#[derive(Debug)]
pub enum FetchRequest {
    /// Gets the height of the blockchain from the server.
    ChainTip(oneshot::Sender<BlockId>),
    /// Gets the specified range of compact blocks from the server (end exclusive).
    CompactBlockRange(oneshot::Sender<Vec<CompactBlock>>, Range<BlockHeight>),
    /// Gets the tree states for a specified block height.
    TreeState(oneshot::Sender<TreeState>, BlockHeight),
    /// Get a full transaction by txid.
    Transaction(oneshot::Sender<(Transaction, BlockHeight)>, TxId),
    /// Get a list of unspent transparent output metadata for a given list of transparent addresses and block range.
    UtxoMetadata(
        oneshot::Sender<Vec<GetAddressUtxosReply>>,
        (Vec<String>, BlockHeight),
    ),
}

/// Gets the height of the blockchain from the server.
///
/// Requires [`crate::client::fetch::fetch`] to be running concurrently, connected via the `fetch_request` channel.
pub async fn get_chain_height(
    fetch_request_sender: UnboundedSender<FetchRequest>,
) -> Result<BlockHeight, ()> {
    let (sender, receiver) = oneshot::channel();
    fetch_request_sender
        .send(FetchRequest::ChainTip(sender))
        .unwrap();
    let chain_tip = receiver.await.unwrap();

    Ok(BlockHeight::from_u32(chain_tip.height as u32))
}

/// Gets the specified range of compact blocks from the server (end exclusive).
///
/// Requires [`crate::client::fetch::fetch`] to be running concurrently, connected via the `fetch_request` channel.
pub async fn get_compact_block_range(
    fetch_request_sender: UnboundedSender<FetchRequest>,
    block_range: Range<BlockHeight>,
) -> Result<Vec<CompactBlock>, ()> {
    let (sender, receiver) = oneshot::channel();
    fetch_request_sender
        .send(FetchRequest::CompactBlockRange(sender, block_range))
        .unwrap();
    let compact_blocks = receiver.await.unwrap();

    Ok(compact_blocks)
}

/// Gets the frontiers for a specified block height.
///
/// Requires [`crate::client::fetch::fetch`] to be running concurrently, connected via the `fetch_request` channel.
pub async fn get_frontiers(
    fetch_request_sender: UnboundedSender<FetchRequest>,
    block_height: BlockHeight,
) -> Result<ChainState, ()> {
    let (sender, receiver) = oneshot::channel();
    fetch_request_sender
        .send(FetchRequest::TreeState(sender, block_height))
        .unwrap();
    let tree_state = receiver.await.unwrap();
    let frontiers = tree_state.to_chain_state().unwrap();

    Ok(frontiers)
}

/// Gets a full transaction for a specified txid.
///
/// Requires [`crate::client::fetch::fetch`] to be running concurrently, connected via the `fetch_request` channel.
pub async fn get_transaction_and_block_height(
    fetch_request_sender: UnboundedSender<FetchRequest>,
    txid: TxId,
) -> Result<(Transaction, BlockHeight), ()> {
    let (sender, receiver) = oneshot::channel();
    fetch_request_sender
        .send(FetchRequest::Transaction(sender, txid))
        .unwrap();
    let transaction_and_block_height = receiver.await.unwrap();

    Ok(transaction_and_block_height)
}

/// Gets unspent transparent output metadata for a list of `transparent addresses` from the specified `start_height`.
///
/// Requires [`crate::client::fetch::fetch`] to be running concurrently, connected via the `fetch_request` channel.
pub async fn get_utxo_metadata(
    fetch_request_sender: UnboundedSender<FetchRequest>,
    transparent_addresses: Vec<String>,
    start_height: BlockHeight,
) -> Result<Vec<GetAddressUtxosReply>, ()> {
    if transparent_addresses.is_empty() {
        panic!("addresses must be non-empty!");
    }

    let (sender, receiver) = oneshot::channel();
    fetch_request_sender
        .send(FetchRequest::UtxoMetadata(
            sender,
            (transparent_addresses, start_height),
        ))
        .unwrap();
    let transparent_output_metadata = receiver.await.unwrap();

    Ok(transparent_output_metadata)
}
