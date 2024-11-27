use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use tokio::sync::mpsc;

use zcash_address::{ToAddress, ZcashAddress};
use zcash_client_backend::proto::service::GetAddressUtxosReply;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::{self, BlockHeight};
use zcash_primitives::legacy::keys::{AccountPubKey, IncomingViewingKey, NonHardenedChildIndex};
use zcash_primitives::legacy::{Script, TransparentAddress};
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::AccountId;

use crate::client::{self, FetchRequest};
use crate::keys::{AddressIndex, TransparentAddressId, TransparentScope};
use crate::primitives::{OutputId, TransparentCoin};
use crate::traits::{SyncTransactions, SyncWallet};

use super::{ADDRESS_GAP_LIMIT, MAX_VERIFICATION_WINDOW};

// ///
// pub(crate) async fn update_coins_outer<W>(
//     wallet: &mut W,
//     fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
//     previous_sync_wallet_height: BlockHeight,
// ) where
//     W: SyncWallet + SyncTransactions,
// {
//     let wallet_addresses: Vec<String> = wallet
//         .get_transparent_addresses()
//         .unwrap()
//         .iter()
//         .map(|(_, address)| address.clone())
//         .collect();

//     if wallet_addresses.is_empty() {
//         return;
//     }

//     let utxo_metadata: Vec<UtxoMetadata> = client::get_utxo_metadata(
//         fetch_request_sender.clone(),
//         wallet_addresses,
//         wallet.get_birthday().unwrap(),
//     )
//     .await
//     .unwrap()
//     .into_iter()
//     .map(|reply| reply.into())
//     .collect();

//     let wallet_transactions = wallet.get_wallet_transactions_mut().unwrap();
//     for utxo in utxo_metadata {
//         if let Some(transaction) = wallet_transactions.get_mut(utxo.txid()) {
//             //
//         }
//     }
// }

///
pub(crate) async fn create_coins<W>(
    addresses: &BTreeMap<TransparentAddressId, String>,
    utxo_metadata: Vec<GetAddressUtxosReply>,
) -> Vec<(BlockHeight, TransparentCoin)> {
    utxo_metadata
        .into_iter()
        .map(|utxo| {
            let key_id = addresses
                .iter()
                .find(|(_, address)| **address == utxo.address)
                .map(|(key_id, _)| key_id.clone())
                .expect("utxo metadata should be derived from an address in the wallet!");
            let txid = TxId::from_bytes(<[u8; 32]>::try_from(&utxo.txid[..]).unwrap());
            let transparent_coin = TransparentCoin::from_parts(
                OutputId::from_parts(txid, utxo.index as usize),
                key_id,
                utxo.address,
                Script(utxo.script),
                u64::try_from(utxo.value_zat).unwrap(),
                false,
            );

            (
                BlockHeight::from_u32(u32::try_from(utxo.height).unwrap()),
                transparent_coin,
            )
        })
        .collect()
}

// TODO: finish doc comment
/// if the new set of all UTXOs does not contain a previously unspent coin in the wallet, the coin must have been spent.
pub(crate) async fn update_spend_status<W>(wallet: &mut W, coin_ids: Vec<OutputId>)
where
    W: SyncWallet + SyncTransactions,
{
    // let coin_ids: Vec<OutputId> = coin_metadata
    //     .iter()
    //     .map(|(_, coin)| coin.output_id())
    //     .collect();
    wallet
        .get_wallet_transactions_mut()
        .unwrap()
        .values_mut()
        .flat_map(|tx| tx.transparent_coins_mut())
        .filter(|coin| !coin_ids.contains(&coin.output_id()))
        .for_each(|coin| {
            coin.set_spent(true);
        });
}

/// Updates the wallet with any previously unknown transparent addresses that are now in use.
/// Also updates the transparent output locators for any newly added addresses.
pub(crate) async fn discover_addresses<P, W>(
    wallet: &mut W,
    consensus_parameters: &P,
    fetch_request_sender: mpsc::UnboundedSender<FetchRequest>,
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    previous_sync_wallet_height: BlockHeight,
) where
    P: consensus::Parameters,
    W: SyncWallet + SyncTransactions,
{
    let mut utxo_metadata_set: Vec<GetAddressUtxosReply> = Vec::new();
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
        let mut utxo_metadata = client::get_utxo_metadata(
            fetch_request_sender.clone(),
            gap_limit_addresses
                .iter()
                .map(|(_, address)| address.clone())
                .collect(),
            previous_sync_wallet_height - MAX_VERIFICATION_WINDOW,
        )
        .await
        .unwrap();
        // tracing::info!("outputs:\n{:#?}", &transparent_output_metadata);

        if utxo_metadata.is_empty() {
            // gap limit satisfied for all scopes of all accounts.
            break;
        }

        // determine if any of these newly derived addresses are used
        add_new_addresses(
            ufvks,
            &mut wallet_addresses,
            &gap_limit_addresses,
            &utxo_metadata,
        );
        // tracing::info!("wallet:\n{:#?}", &wallet_addresses);

        utxo_metadata_set.append(&mut utxo_metadata);
    }

    update_addresses(wallet, wallet_addresses);
    update_coins(wallet, utxo_metadata_set);
}

fn update_addresses<W>(wallet: &mut W, transparent_addresses: Vec<(TransparentAddressId, String)>)
where
    W: SyncWallet,
{
    let wallet_addresses = wallet.get_transparent_addresses_mut().unwrap();
    for (id, address) in transparent_addresses {
        wallet_addresses.insert(id, address);
    }
}

/// For each scope in each account, add all `gap_limit_addresses` up to the highest index
/// address contained in `transparent_output_metadata` to `wallet_addresses`.
fn add_new_addresses(
    ufvks: &HashMap<AccountId, UnifiedFullViewingKey>,
    wallet_addresses: &mut Vec<(TransparentAddressId, String)>,
    gap_limit_addresses: &[(TransparentAddressId, String)],
    utxo_metadata: &[GetAddressUtxosReply],
) {
    let utxo_metadata_addresses: Vec<&str> = utxo_metadata
        .iter()
        .map(|utxo| utxo.address.as_str())
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
                        .find(|(_, address)| utxo_metadata_addresses.contains(&address.as_str()))
                {
                    Some(id.address_index() as usize)
                } else {
                    None
                };

                if let Some(address_index) = highest_new_address_index {
                    gap_limit_addresses_by_scope.truncate(address_index + 1);
                    wallet_addresses.append(&mut gap_limit_addresses_by_scope);
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

                gap_limit_addresses.extend(derive_addresses_by_scope(
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

fn derive_addresses_by_scope<P>(
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
        TransparentScope::External => derive_addresses(
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
        TransparentScope::Internal => derive_addresses(
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
        TransparentScope::Refund => derive_addresses(
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

fn derive_addresses<F, P>(
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
