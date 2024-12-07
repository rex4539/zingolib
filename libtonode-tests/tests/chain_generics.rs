use tokio::runtime::Runtime;
use zingolib::testutils::{
    chain_generics::{fixtures, libtonode::LibtonodeEnvironment},
    int_to_pooltype, int_to_shieldedprotocol,
};

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(8))]
    #[test]
    fn single_sufficient_send_libtonode(send_value in 0..50_000u64, change_value in 0..10_000u64, sender_protocol in 1..2, receiver_pool in 0..2) {
        Runtime::new().unwrap().block_on(async {
            fixtures::single_sufficient_send::<LibtonodeEnvironment>(int_to_shieldedprotocol(sender_protocol), int_to_pooltype(receiver_pool), send_value, change_value, true).await;
        });
     }
    #[test]
    fn single_sufficient_send_0_change_libtonode(send_value in 0..50_000u64, sender_protocol in 1..2, receiver_pool in 0..2) {
        Runtime::new().unwrap().block_on(async {
            fixtures::single_sufficient_send::<LibtonodeEnvironment>(int_to_shieldedprotocol(sender_protocol), int_to_pooltype(receiver_pool), send_value, 0, true).await;
        });
     }
}

mod chain_generics {
    use zcash_client_backend::PoolType::Shielded;
    use zcash_client_backend::PoolType::Transparent;
    use zcash_client_backend::ShieldedProtocol::Orchard;
    use zcash_client_backend::ShieldedProtocol::Sapling;

    use zingolib::testutils::chain_generics::fixtures;
    use zingolib::testutils::chain_generics::libtonode::LibtonodeEnvironment;

    #[tokio::test]
    async fn generate_a_range_of_value_transfers() {
        fixtures::create_various_value_transfers::<LibtonodeEnvironment>().await;
    }
    #[tokio::test]
    async fn send_40_000_to_transparent() {
        fixtures::send_value_to_pool::<LibtonodeEnvironment>(40_000, Transparent).await;
    }
    #[tokio::test]
    async fn send_40_000_to_sapling() {
        fixtures::send_value_to_pool::<LibtonodeEnvironment>(40_000, Shielded(Sapling)).await;
    }
    #[tokio::test]
    async fn send_40_000_to_orchard() {
        fixtures::send_value_to_pool::<LibtonodeEnvironment>(40_000, Shielded(Orchard)).await;
    }

    #[tokio::test]
    async fn propose_and_broadcast_40_000_to_transparent() {
        fixtures::propose_and_broadcast_value_to_pool::<LibtonodeEnvironment>(40_000, Transparent)
            .await;
    }
    #[tokio::test]
    async fn propose_and_broadcast_40_000_to_sapling() {
        fixtures::propose_and_broadcast_value_to_pool::<LibtonodeEnvironment>(
            40_000,
            Shielded(Sapling),
        )
        .await;
    }
    #[tokio::test]
    async fn propose_and_broadcast_40_000_to_orchard() {
        fixtures::propose_and_broadcast_value_to_pool::<LibtonodeEnvironment>(
            40_000,
            Shielded(Orchard),
        )
        .await;
    }
    #[tokio::test]
    async fn send_shield_cycle() {
        fixtures::send_shield_cycle::<LibtonodeEnvironment>(1).await;
    }
    #[tokio::test]
    async fn ignore_dust_inputs() {
        fixtures::ignore_dust_inputs::<LibtonodeEnvironment>().await;
    }
    #[ignore]
    #[tokio::test]
    async fn send_grace_dust() {
        fixtures::send_grace_dust::<LibtonodeEnvironment>().await;
    }
    #[ignore]
    #[tokio::test]
    async fn change_required() {
        fixtures::change_required::<LibtonodeEnvironment>().await;
    }
    #[ignore]
    #[tokio::test]
    async fn send_required_dust() {
        fixtures::send_required_dust::<LibtonodeEnvironment>().await;
    }
    #[tokio::test]
    async fn note_selection_order() {
        fixtures::note_selection_order::<LibtonodeEnvironment>().await;
    }
    #[tokio::test]
    async fn simpool_insufficient_1_sapling_to_transparent() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Sapling,
            Transparent,
            1,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_1_sapling_to_sapling() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Sapling,
            Shielded(Sapling),
            1,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_1_sapling_to_orchard() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Sapling,
            Shielded(Orchard),
            1,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_1_orchard_to_transparent() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Orchard,
            Transparent,
            1,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_1_orchard_to_sapling() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Orchard,
            Shielded(Sapling),
            1,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_1_orchard_to_orchard() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Orchard,
            Shielded(Orchard),
            1,
        )
        .await
    }
    #[tokio::test]
    async fn simpool_insufficient_10_000_sapling_to_transparent() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Sapling,
            Transparent,
            10_000,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_10_000_sapling_to_sapling() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Sapling,
            Shielded(Sapling),
            10_000,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_10_000_sapling_to_orchard() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Sapling,
            Shielded(Orchard),
            10_000,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_10_000_orchard_to_transparent() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Orchard,
            Transparent,
            10_000,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_10_000_orchard_to_sapling() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Orchard,
            Shielded(Sapling),
            10_000,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_insufficient_10_000_orchard_to_orchard() {
        fixtures::shpool_to_pool_insufficient_error::<LibtonodeEnvironment>(
            Orchard,
            Shielded(Orchard),
            10_000,
        )
        .await;
    }
    #[tokio::test]
    async fn simpool_no_fund_1_000_000_to_transparent() {
        fixtures::to_pool_unfunded_error::<LibtonodeEnvironment>(Transparent, 1_000_000).await;
    }
    #[tokio::test]
    async fn simpool_no_fund_1_000_000_to_sapling() {
        fixtures::to_pool_unfunded_error::<LibtonodeEnvironment>(Shielded(Sapling), 1_000_000)
            .await;
    }
    #[tokio::test]
    async fn simpool_no_fund_1_000_000_to_orchard() {
        fixtures::to_pool_unfunded_error::<LibtonodeEnvironment>(Shielded(Orchard), 1_000_000)
            .await;
    }
}
