//! lightclient functions with added assertions. used for tests.

use crate::lightclient::LightClient;
use crate::testutils::assertions::assert_recipient_total_lte_to_proposal_total;
use crate::testutils::assertions::lookup_fees_with_proposal_check;
use crate::testutils::chain_generics::conduct_chain::ConductChain;
use crate::testutils::lightclient::from_inputs;
use crate::testutils::lightclient::get_base_address;
use crate::testutils::lightclient::lookup_statuses;
use std::num::NonZeroU32;
use zcash_client_backend::data_api::WalletRead as _;
use zcash_client_backend::PoolType;
use zingo_status::confirmation_status::ConfirmationStatus;

/// this function handles inputs and their lifetimes to create a proposal
pub async fn to_clients_proposal(
    sender: &LightClient,
    sends: &Vec<(&LightClient, PoolType, u64, Option<&str>)>,
) -> zcash_client_backend::proposal::Proposal<
    zcash_primitives::transaction::fees::zip317::FeeRule,
    zcash_client_backend::wallet::NoteId,
> {
    let mut subraw_receivers = vec![];
    for (recipient, pooltype, amount, memo_str) in sends.clone() {
        let address = get_base_address(recipient, pooltype).await;
        subraw_receivers.push((address, amount, memo_str));
    }

    let raw_receivers = subraw_receivers
        .iter()
        .map(|(address, amount, opt_memo)| (address.as_str(), *amount, *opt_memo))
        .collect();

    from_inputs::propose(sender, raw_receivers).await.unwrap()
}

/// sends to any combo of recipient clients checks that each recipient also recieved the expected balances
/// test-only generic
/// NOTICE this function bumps the chain and syncs the client
/// only compatible with zip317
/// returns the total fee for the transfer
/// test_mempool can be enabled when the test harness supports it: TBI
pub async fn propose_send_bump_sync_all_recipients<CC>(
    environment: &mut CC,
    sender: &LightClient,
    sends: Vec<(&LightClient, PoolType, u64, Option<&str>)>,
    test_mempool: bool,
) -> u64
where
    CC: ConductChain,
{
    let proposal = to_clients_proposal(sender, &sends).await;

    let send_height = sender
        .wallet
        .transaction_context
        .transaction_metadata_set
        .read()
        .await
        .get_target_and_anchor_heights(NonZeroU32::MIN)
        .expect("sender has a target height")
        .expect("sender has a target height")
        .0;

    let txids = sender
        .complete_and_broadcast_stored_proposal()
        .await
        .unwrap();

    // digesting the calculated transaction
    // this step happens after transaction is recorded locally, but before learning anything about whether the server accepted it
    let recorded_fee = *lookup_fees_with_proposal_check(sender, &proposal, &txids)
        .await
        .first()
        .expect("one transaction proposed")
        .as_ref()
        .expect("record is ok");

    dbg!(
        crate::grpc_connector::get_latest_block(sender.config.lightwalletd_uri.read().unwrap())
            .await
            .unwrap()
            .height
    );

    lookup_statuses(sender, txids.clone()).await.map(|status| {
        assert_eq!(
            status,
            Some(ConfirmationStatus::Transmitted(send_height.into()))
        );
    });

    let send_ua_id = sender.do_addresses().await[0]["address"].clone();

    if test_mempool {
        // mempool scan shows the same
        sender.do_sync(false).await.unwrap();

        // let the mempool monitor get a chance
        // to listen
        tokio::time::sleep(std::time::Duration::from_secs(6)).await;

        lookup_fees_with_proposal_check(sender, &proposal, &txids)
            .await
            .first()
            .expect("one transaction to be proposed")
            .as_ref()
            .expect("record to be ok");

        lookup_statuses(sender, txids.clone()).await.map(|status| {
            assert!(matches!(status, Some(ConfirmationStatus::Mempool(_))));
        });

        // TODO: distribute receivers
        for (recipient, _, _, _) in sends.clone() {
            if send_ua_id != recipient.do_addresses().await[0]["address"].clone() {
                recipient.do_sync(false).await.unwrap();
                let records = &recipient
                    .wallet
                    .transaction_context
                    .transaction_metadata_set
                    .read()
                    .await
                    .transaction_records_by_id;
                for txid in &txids {
                    let record = records.get(txid).expect("recipient must recognize txid");
                    assert_eq!(record.status, ConfirmationStatus::Mempool(send_height),)
                }
            }
        }
    }

    environment.bump_chain().await;
    // chain scan shows the same
    sender.do_sync(false).await.unwrap();
    lookup_fees_with_proposal_check(sender, &proposal, &txids)
        .await
        .first()
        .expect("one transaction to be proposed")
        .as_ref()
        .expect("record to be ok");

    lookup_statuses(sender, txids.clone()).await.map(|status| {
        assert!(matches!(status, Some(ConfirmationStatus::Confirmed(_))));
    });

    for (recipient, _, _, _) in sends {
        if send_ua_id != recipient.do_addresses().await[0]["address"].clone() {
            recipient.do_sync(false).await.unwrap();
            assert_recipient_total_lte_to_proposal_total(recipient, &proposal, &txids).await;
        }
    }
    recorded_fee
}

/// a test-only generic version of shield that includes assertions that the proposal was fulfilled
/// NOTICE this function bumps the chain and syncs the client
/// only compatible with zip317
/// returns the total fee for the transfer
pub async fn assure_propose_shield_bump_sync<CC>(
    environment: &mut CC,
    client: &LightClient,
    test_mempool: bool,
) -> Result<u64, String>
where
    CC: ConductChain,
{
    let proposal = client.propose_shield().await.map_err(|e| e.to_string())?;

    let send_height = client
        .wallet
        .transaction_context
        .transaction_metadata_set
        .read()
        .await
        .get_target_and_anchor_heights(NonZeroU32::MIN)
        .expect("sender has a target height")
        .expect("sender has a target height")
        .0;

    let txids = client
        .complete_and_broadcast_stored_proposal()
        .await
        .unwrap();

    // digesting the calculated transaction
    let recorded_fee = *lookup_fees_with_proposal_check(client, &proposal, &txids)
        .await
        .first()
        .expect("one transaction proposed")
        .as_ref()
        .expect("record is ok");

    lookup_statuses(client, txids.clone()).await.map(|status| {
        assert_eq!(
            status,
            Some(ConfirmationStatus::Transmitted(send_height.into()))
        );
    });

    if test_mempool {
        // mempool scan shows the same
        client.do_sync(false).await.unwrap();
        lookup_fees_with_proposal_check(client, &proposal, &txids)
            .await
            .first()
            .expect("one transaction proposed")
            .as_ref()
            .expect("record is ok");

        lookup_statuses(client, txids.clone()).await.map(|status| {
            assert!(matches!(status, Some(ConfirmationStatus::Mempool(_))));
        });
    }

    environment.bump_chain().await;
    // chain scan shows the same
    client.do_sync(false).await.unwrap();
    lookup_fees_with_proposal_check(client, &proposal, &txids)
        .await
        .first()
        .expect("one transaction proposed")
        .as_ref()
        .expect("record is ok");

    lookup_statuses(client, txids.clone()).await.map(|status| {
        assert!(matches!(status, Some(ConfirmationStatus::Confirmed(_))));
    });

    Ok(recorded_fee)
}
