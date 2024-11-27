use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use tokio::sync::mpsc;

use zcash_address::{ToAddress, ZcashAddress};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::{self, BlockHeight};
use zcash_primitives::legacy::keys::{AccountPubKey, IncomingViewingKey, NonHardenedChildIndex};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::AccountId;

use crate::client::{get_transparent_address_transactions, FetchRequest};
use crate::keys::{AddressIndex, TransparentAddressId, TransparentScope};
use crate::traits::SyncWallet;

const ADDRESS_GAP_LIMIT: usize = 20;

/// Discovers all addresses in use by the wallet and returns locators for the relevant transactions to scan transparent
/// bundles.
pub(crate) async fn update_addresses_and_locators<P, W>(
    wallet: &mut W,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    consensus_parameters: &P,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    chain_height: BlockHeight,
) where
    P: consensus::Parameters,
    W: SyncWallet,
{
    let wallet_birthday = wallet.get_birthday().unwrap();
    let wallet_addresses = wallet.get_transparent_addresses_mut().unwrap();
    let block_range = Range {
        start: wallet_birthday,
        end: chain_height + 1,
    };
    let mut locators: BTreeSet<(BlockHeight, TxId)> = BTreeSet::new();

    for (account_id, ufvk) in ufvks {
        if let Some(account_pubkey) = ufvk.transparent() {
            for scope in [
                TransparentScope::External,
                TransparentScope::Internal,
                TransparentScope::Refund,
            ] {
                let mut address_index = 0;
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

fn encode_address<P>(consensus_parameters: &P, address: TransparentAddress) -> String
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
