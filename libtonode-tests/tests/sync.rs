use tempfile::TempDir;
use testvectors::seeds::HOSPITAL_MUSEUM_SEED;
use zingo_netutils::GrpcConnector;
use zingo_sync::sync::sync;
use zingolib::{
    config::{construct_lightwalletd_uri, load_clientconfig, DEFAULT_LIGHTWALLETD_SERVER},
    get_base_address_macro,
    lightclient::LightClient,
    testutils::{lightclient::from_inputs, scenarios},
    wallet::WalletBase,
};

#[ignore = "too slow, and flakey"]
#[tokio::test]
async fn sync_mainnet_test() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Ring to work as a default");
    tracing_subscriber::fmt().init();

    let uri = construct_lightwalletd_uri(Some(DEFAULT_LIGHTWALLETD_SERVER.to_string()));
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path().to_path_buf();
    let config = load_clientconfig(
        uri.clone(),
        Some(temp_path),
        zingolib::config::ChainType::Mainnet,
        true,
    )
    .unwrap();
    let mut lightclient = LightClient::create_from_wallet_base_async(
        WalletBase::from_string(HOSPITAL_MUSEUM_SEED.to_string()),
        &config,
        2_650_318,
        true,
    )
    .await
    .unwrap();

    let client = GrpcConnector::new(uri).get_client().await.unwrap();

    sync(client, &config.chain, &mut lightclient.wallet)
        .await
        .unwrap();

    // dbg!(lightclient.wallet.wallet_blocks);
    // dbg!(lightclient.wallet.nullifier_map);
    dbg!(lightclient.wallet.sync_state);
}

#[ignore = "hangs"]
#[tokio::test]
async fn sync_test() {
    tracing_subscriber::fmt().init();

    let (_regtest_manager, _cph, faucet, mut recipient, _txid) =
        scenarios::faucet_funded_recipient_default(5_000_000).await;
    from_inputs::quick_send(
        &faucet,
        vec![(
            &get_base_address_macro!(&recipient, "transparent"),
            100_000,
            None,
        )],
    )
    .await
    .unwrap();
    // from_inputs::quick_send(
    //     &recipient,
    //     vec![(
    //         &get_base_address_macro!(&faucet, "unified"),
    //         100_000,
    //         Some("Outgoing decrypt test"),
    //     )],
    // )
    // .await
    // .unwrap();

    // increase_server_height(&regtest_manager, 1).await;
    // recipient.do_sync(false).await.unwrap();
    // recipient.quick_shield().await.unwrap();
    // increase_server_height(&regtest_manager, 1).await;

    let uri = recipient.config().lightwalletd_uri.read().unwrap().clone();
    let client = GrpcConnector::new(uri).get_client().await.unwrap();
    sync(
        client,
        &recipient.config().chain.clone(),
        &mut recipient.wallet,
    )
    .await
    .unwrap();

    // dbg!(&recipient.wallet.wallet_transactions);
    // dbg!(recipient.wallet.wallet_blocks());
    // dbg!(recipient.wallet.nullifier_map());
    // dbg!(recipient.wallet.outpoint_map());
    // dbg!(recipient.wallet.sync_state());
}
