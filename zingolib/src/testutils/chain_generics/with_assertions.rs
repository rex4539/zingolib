//! lightclient functions with added assertions. used for tests.

use crate::lightclient::LightClient;
use crate::testutils::assertions::lookup_fees_with_proposal_check;
use crate::testutils::chain_generics::conduct_chain::ConductChain;
use crate::testutils::lightclient::from_inputs;
use crate::testutils::lightclient::get_base_address;
use crate::testutils::lightclient::lookup_statuses;
use nonempty::NonEmpty;
use zcash_client_backend::proposal::Proposal;
use zcash_client_backend::PoolType;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::transaction::TxId;
use zingo_status::confirmation_status::ConfirmationStatus;

/// this function handles inputs and their lifetimes to create a proposal
pub async fn to_clients_proposal(
    sender: &LightClient,
    sends: &[(&LightClient, PoolType, u64, Option<&str>)],
) -> zcash_client_backend::proposal::Proposal<
    zcash_primitives::transaction::fees::zip317::FeeRule,
    zcash_client_backend::wallet::NoteId,
> {
    let mut subraw_receivers = vec![];
    for (recipient, pooltype, amount, memo_str) in sends {
        let address = get_base_address(recipient, *pooltype).await;
        subraw_receivers.push((address, amount, memo_str));
    }

    let raw_receivers = subraw_receivers
        .iter()
        .map(|(address, amount, opt_memo)| (address.as_str(), **amount, **opt_memo))
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
    sender.do_sync(false).await.unwrap();
    let proposal = to_clients_proposal(sender, &sends).await;

    let txids = sender
        .complete_and_broadcast_stored_proposal()
        .await
        .unwrap();

    follow_proposal(
        environment,
        sender,
        sends
            .iter()
            .map(|(recipient, _, _, _)| *recipient)
            .collect(),
        &proposal,
        txids,
        test_mempool,
    )
    .await
    .unwrap()
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

    let txids = client
        .complete_and_broadcast_stored_proposal()
        .await
        .unwrap();

    follow_proposal(environment, client, vec![], &proposal, txids, test_mempool).await
}

/// given a just-broadcast proposal, confirms that it achieves all expected checkpoints.
pub async fn follow_proposal<CC, NoteRef>(
    environment: &mut CC,
    sender: &LightClient,
    recipients: Vec<&LightClient>,
    proposal: &Proposal<zcash_primitives::transaction::fees::zip317::FeeRule, NoteRef>,
    txids: NonEmpty<TxId>,
    test_mempool: bool,
) -> Result<u64, String>
where
    CC: ConductChain,
{
    let server_height_at_send = BlockHeight::from(
        crate::grpc_connector::get_latest_block(environment.lightserver_uri().unwrap())
            .await
            .unwrap()
            .height as u32,
    );

    // digesting the calculated transaction
    // this step happens after transaction is recorded locally, but before learning anything about whether the server accepted it
    let recorded_fee = *lookup_fees_with_proposal_check(sender, &proposal, &txids)
        .await
        .first()
        .expect("one transaction proposed")
        .as_ref()
        .expect("record is ok");

    lookup_statuses(sender, txids.clone()).await.map(|status| {
        assert_eq!(
            status,
            Some(ConfirmationStatus::Transmitted(server_height_at_send + 1))
        );
    });

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
            assert_eq!(
                status,
                Some(ConfirmationStatus::Mempool(server_height_at_send + 1)),
            )
        });

        for recipient in recipients.clone() {
            recipient.do_sync(false).await.unwrap();

            lookup_statuses(recipient, txids.clone())
                .await
                .map(|status| {
                    assert_eq!(
                        status,
                        Some(ConfirmationStatus::Mempool(server_height_at_send + 1)),
                    )
                });
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

    for recipient in recipients {
        recipient.do_sync(false).await.unwrap();
        lookup_statuses(recipient, txids.clone())
            .await
            .map(|status| {
                assert_eq!(
                    status,
                    Some(ConfirmationStatus::Confirmed(server_height_at_send + 1)),
                )
            });
    }

    Ok(recorded_fee)
}
