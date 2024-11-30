use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use tokio::sync::mpsc;

use zcash_address::{ToAddress, ZcashAddress};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::{self, BlockHeight};
use zcash_primitives::legacy::keys::{AccountPubKey, IncomingViewingKey, NonHardenedChildIndex};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::zip32::AccountId;

use crate::client::{get_transparent_address_transactions, FetchRequest};
use crate::keys::{AddressIndex, TransparentAddressId, TransparentScope};
use crate::primitives::Locator;
use crate::traits::SyncWallet;

use super::MAX_VERIFICATION_WINDOW;

const ADDRESS_GAP_LIMIT: usize = 20;

/// Discovers all addresses in use by the wallet and returns locators for any new relevant transactions to scan transparent
/// bundles.
/// `wallet_height` should be the value before updating scan ranges. i.e. the wallet height as of previous sync.
pub(crate) async fn update_addresses_and_locators<P, W>(
    wallet: &mut W,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    consensus_parameters: &P,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    wallet_height: BlockHeight,
    chain_height: BlockHeight,
) where
    P: consensus::Parameters,
    W: SyncWallet,
{
    let wallet_addresses = wallet.get_transparent_addresses_mut().unwrap();
    let mut locators: BTreeSet<Locator> = BTreeSet::new();
    let block_range = Range {
        start: wallet_height + 1 - MAX_VERIFICATION_WINDOW,
        end: chain_height + 1,
    };

    // find locators for any new transactions relevant to known addresses
    for address in wallet_addresses.values() {
        let transactions = get_transparent_address_transactions(
            fetch_request_sender.clone(),
            address.clone(),
            block_range.clone(),
        )
        .await
        .unwrap();

        transactions.iter().for_each(|(height, tx)| {
            locators.insert((height.clone(), tx.txid()));
        });
    }

    // discover new addresses and find locators for relevant transactions
    for (account_id, ufvk) in ufvks {
        if let Some(account_pubkey) = ufvk.transparent() {
            for scope in [
                TransparentScope::External,
                TransparentScope::Internal,
                TransparentScope::Refund,
            ] {
                // start with the first address index previously unused by the wallet
                let mut address_index = if let Some(id) = wallet_addresses
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
                let mut unused_address_count: usize = 0;
                let mut addresses: Vec<(TransparentAddressId, String)> = Vec::new();

                while unused_address_count < ADDRESS_GAP_LIMIT {
                    let address_id =
                        TransparentAddressId::from_parts(*account_id, scope, address_index);
                    let address = derive_address(consensus_parameters, account_pubkey, address_id);
                    addresses.push((address_id, address.clone()));

                    let transactions = get_transparent_address_transactions(
                        fetch_request_sender.clone(),
                        address,
                        block_range.clone(),
                    )
                    .await
                    .unwrap();

                    if transactions.is_empty() {
                        unused_address_count += 1;
                    } else {
                        transactions.iter().for_each(|(height, tx)| {
                            locators.insert((height.clone(), tx.txid()));
                        });
                        unused_address_count = 0;
                    }

                    address_index += 1;
                }

                addresses.truncate(addresses.len() - ADDRESS_GAP_LIMIT);
                addresses.into_iter().for_each(|(id, address)| {
                    wallet_addresses.insert(id, address);
                });
            }
        }
    }

    wallet
        .get_sync_state_mut()
        .unwrap()
        .locators_mut()
        .append(&mut locators);
}

fn derive_address<P>(
    consensus_parameters: &P,
    account_pubkey: &AccountPubKey,
    address_id: TransparentAddressId,
) -> String
where
    P: consensus::Parameters,
{
    let address = match address_id.scope() {
        TransparentScope::External => {
            derive_external_address(account_pubkey, address_id.address_index())
        }
        TransparentScope::Internal => {
            derive_internal_address(account_pubkey, address_id.address_index())
        }
        TransparentScope::Refund => {
            derive_refund_address(account_pubkey, address_id.address_index())
        }
    };

    encode_address(consensus_parameters, address)
}

fn derive_external_address(
    account_pubkey: &AccountPubKey,
    address_index: AddressIndex,
) -> TransparentAddress {
    account_pubkey
        .derive_external_ivk()
        .unwrap()
        .derive_address(
            NonHardenedChildIndex::from_index(address_index)
                .expect("all non-hardened address indexes in use!"),
        )
        .unwrap()
}

fn derive_internal_address(
    account_pubkey: &AccountPubKey,
    address_index: AddressIndex,
) -> TransparentAddress {
    account_pubkey
        .derive_internal_ivk()
        .unwrap()
        .derive_address(
            NonHardenedChildIndex::from_index(address_index)
                .expect("all non-hardened address indexes in use!"),
        )
        .unwrap()
}

fn derive_refund_address(
    account_pubkey: &AccountPubKey,
    address_index: AddressIndex,
) -> TransparentAddress {
    account_pubkey
        .derive_ephemeral_ivk()
        .unwrap()
        .derive_ephemeral_address(
            NonHardenedChildIndex::from_index(address_index)
                .expect("all non-hardened address indexes in use!"),
        )
        .unwrap()
}

pub(crate) fn encode_address<P>(consensus_parameters: &P, address: TransparentAddress) -> String
where
    P: consensus::Parameters,
{
    let zcash_address = match address {
        TransparentAddress::PublicKeyHash(data) => {
            ZcashAddress::from_transparent_p2pkh(consensus_parameters.network_type(), data)
        }
        TransparentAddress::ScriptHash(data) => {
            ZcashAddress::from_transparent_p2sh(consensus_parameters.network_type(), data)
        }
    };
    zcash_address.to_string()
}

// TODO: process memo encoded address indexes.
// 1. return any memo address ids from scan in ScanResults
// 2. derive the addresses up to that index, add to wallet addresses and send them to GetTaddressTxids
// 3. for each transaction returned:
// a) if the tx is in a range that is not scanned, add locator to sync_state
// b) if the range is scanned and the tx is already in the wallet, rescan the zcash transaction transparent bundles in
// the wallet transaction
// c) if the range is scanned and the tx does not exist in the wallet, fetch the compact block if its not in the wallet
// and scan the transparent bundles
