//! lightclient functions with added assertions. used for tests.

use crate::lightclient::LightClient;
use crate::testutils::assertions::compare_fee;
use crate::testutils::assertions::for_each_proposed_record;
use crate::testutils::assertions::ProposalToTransactionRecordComparisonError;
use crate::testutils::chain_generics::conduct_chain::ConductChain;
use crate::testutils::lightclient::from_inputs;
use crate::testutils::lightclient::get_base_address;
use crate::wallet::notes::query::OutputQuery;
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
    println!("following proposal, preparing to unwind if an assertion fails.");

    let server_height_at_send = BlockHeight::from(
        crate::grpc_connector::get_latest_block(environment.lightserver_uri().unwrap())
            .await
            .unwrap()
            .height as u32,
    );

    // check that each record has the expected fee and status, returning the fee
    let (sender_recorded_fees, sender_recorded_outputs): (Vec<u64>, Vec<u64>) =
        for_each_proposed_record(
            sender,
            proposal,
            &txids,
            server_height_at_send + 1,
            |records, record, step, expected_height| {
                assert_eq!(
                    record.status,
                    ConfirmationStatus::Transmitted(expected_height)
                );
                (
                    compare_fee(records, record, step),
                    record.query_sum_value(OutputQuery::any()),
                )
            },
        )
        .await
        .into_iter()
        .map(|stepwise_result| {
            stepwise_result
                .map(|(fee_comparison_result, sender_output_value)| {
                    (fee_comparison_result.unwrap(), sender_output_value)
                })
                .unwrap()
        })
        .unzip();

    let option_recipient_mempool_outputs = if test_mempool {
        // mempool scan shows the same
        sender.do_sync(false).await.unwrap();

        // let the mempool monitor get a chance
        // to listen
        tokio::time::sleep(std::time::Duration::from_secs(6)).await;

        // check that each record has the expected fee and status, returning the fee and outputs
        let (sender_mempool_fees, sender_mempool_outputs): (Vec<u64>, Vec<u64>) =
            for_each_proposed_record(
                sender,
                proposal,
                &txids,
                server_height_at_send + 1,
                |records, record, step, expected_height| {
                    assert_eq!(record.status, ConfirmationStatus::Mempool(expected_height));
                    (
                        compare_fee(records, record, step),
                        record.query_sum_value(OutputQuery::any()),
                    )
                },
            )
            .await
            .into_iter()
            .map(|stepwise_result| {
                stepwise_result
                    .map(|(fee_comparison_result, sender_output_value)| {
                        (fee_comparison_result.unwrap(), sender_output_value)
                    })
                    .unwrap()
            })
            .unzip();

        assert_eq!(sender_mempool_fees, sender_recorded_fees);
        assert_eq!(sender_mempool_outputs, sender_recorded_outputs);

        let mut recipients_mempool_outputs = vec![];
        for recipient in recipients.clone() {
            recipient.do_sync(false).await.unwrap();

            // check that each record has the status, returning the output value
            let recipient_mempool_outputs: Vec<u64> = for_each_proposed_record(
                sender,
                proposal,
                &txids,
                server_height_at_send + 1,
                |records, record, step, expected_height| {
                    assert_eq!(
                        record.status,
                        ConfirmationStatus::Confirmed(expected_height)
                    );
                    record.query_sum_value(OutputQuery::any())
                },
            )
            .await
            .into_iter()
            .map(|stepwise_result| stepwise_result.unwrap())
            .collect();
            recipients_mempool_outputs.push(recipient_mempool_outputs);
        }
        Some(recipients_mempool_outputs)
    } else {
        None
    };

    environment.bump_chain().await;
    // chain scan shows the same
    sender.do_sync(false).await.unwrap();

    // check that each record has the expected fee and status, returning the fee and outputs
    let (sender_confirmed_fees, sender_confirmed_outputs): (Vec<u64>, Vec<u64>) =
        for_each_proposed_record(
            sender,
            proposal,
            &txids,
            server_height_at_send + 1,
            |records, record, step, expected_height| {
                assert_eq!(
                    record.status,
                    ConfirmationStatus::Confirmed(expected_height)
                );
                (
                    compare_fee(records, record, step),
                    record.query_sum_value(OutputQuery::any()),
                )
            },
        )
        .await
        .into_iter()
        .map(|stepwise_result| {
            stepwise_result
                .map(|(fee_comparison_result, sender_output_value)| {
                    (fee_comparison_result.unwrap(), sender_output_value)
                })
                .unwrap()
        })
        .unzip();

    assert_eq!(sender_confirmed_fees, sender_recorded_fees);
    assert_eq!(sender_confirmed_outputs, sender_recorded_outputs);

    let mut recipients_confirmed_outputs = vec![];
    for recipient in recipients {
        recipient.do_sync(false).await.unwrap();

        // check that each record has the status, returning the output value
        let recipient_confirmed_outputs: Vec<u64> = for_each_proposed_record(
            sender,
            proposal,
            &txids,
            server_height_at_send + 1,
            |records, record, step, expected_height| {
                assert_eq!(
                    record.status,
                    ConfirmationStatus::Confirmed(expected_height)
                );
                record.query_sum_value(OutputQuery::any())
            },
        )
        .await
        .into_iter()
        .map(|stepwise_result| stepwise_result.unwrap())
        .collect();
        recipients_confirmed_outputs.push(recipient_confirmed_outputs);
    }

    option_recipient_mempool_outputs.inspect(|recipient_mempool_outputs| {
        assert_eq!(recipients_confirmed_outputs, *recipient_mempool_outputs);
    });

    Ok(sender_recorded_fees.iter().sum())
}
