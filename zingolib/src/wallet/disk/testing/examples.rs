use zcash_client_backend::PoolType;
use zcash_client_backend::ShieldedProtocol;

use super::super::LightWallet;

/// ExampleWalletNetworkCase sorts first by Network, then seed, then last saved version.
/// It is public so that any consumer can select and load any example wallet.
#[non_exhaustive]
#[derive(Clone)]
pub enum NetworkSeedVersion {
    /// /
    Regtest(RegtestSeedVersion),
    /// /
    Testnet(TestnetSeedVersion),
    /// Mainnet is a live chain
    Mainnet(MainnetSeedVersion),
}

#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum MainnetSeedVersion {
    /// this is a mainnet wallet originally called missing_data_test
    VillageTarget(VillageTargetVersion),
    /// empty mainnet wallet
    HotelHumor(HotelHumorVersion),
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum VillageTargetVersion {
    /// wallet was last saved in this serialization version
    V28,
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum HotelHumorVersion {
    /// wallet was last saved in this serialization version
    Gf0aaf9347,
    /// this wallet was funded with 0.01 sapling fr fr fr
    /// latest version of the wallet, with most up-to-date witness tree. git can tell more about when it was saved.
    Latest,
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum TestnetSeedVersion {
    /// this testnet wallet was generated at the beginning of serialization v28 with
    /// --seed "chimney better bulb horror rebuild whisper improve intact letter giraffe brave rib appear bulk aim burst snap salt hill sad merge tennis phrase raise"
    /// with 3 addresses containing all receivers.
    /// including orchard and sapling transactions
    ChimneyBetter(ChimneyBetterVersion),
    /// This is a testnet seed, generated by fluidvanadium at the beginning of owls (this example wallet enumeration)
    MobileShuffle(MobileShuffleVersion),
    /// todo
    GloryGoddess,
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum ChimneyBetterVersion {
    /// wallet was last saved in this serialization version
    V26,
    /// wallet was last saved in this serialization version
    V27,
    /// wallet was last saved in this serialization version
    V28,
    /// wallet was last saved at this commit
    Latest,
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum MobileShuffleVersion {
    /// wallet was last saved by the code in this commit
    Gab72a38b,
    /// this wallet was synced in this version. does it have a bunch of taz scattered around different addresses?
    G93738061a,
    /// most recent chain state added to the wallet
    Latest,
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum RegtestSeedVersion {
    /// this is a regtest wallet originally called old_wallet_reorg_test_wallet
    HospitalMuseum(HospitalMuseumVersion),
    /// this is a regtest wallet originally called v26/sap_only
    AbandonAbandon(AbandonAbandonVersion),
    /// another regtest wallet
    AbsurdAmount(AbsurdAmountVersion),
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum HospitalMuseumVersion {
    /// wallet was last saved in this serialization version
    V27,
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum AbandonAbandonVersion {
    /// wallet was last saved in this serialization version
    V26,
}
#[allow(missing_docs)] // described in parent enum
#[non_exhaustive]
#[derive(Clone)]
pub enum AbsurdAmountVersion {
    /// existing receivers?
    OrchAndSapl,
    /// existing receivers?
    OrchOnly,
}

impl NetworkSeedVersion {
    /// loads test wallets
    /// this function can be improved by a macro. even better would be to derive directly from the enum.
    /// loads any one of the test wallets included in the examples
    pub async fn load_example_wallet(&self) -> LightWallet {
        match self {
            NetworkSeedVersion::Regtest(seed) => match seed {
                RegtestSeedVersion::HospitalMuseum(version) => match version {
                    HospitalMuseumVersion::V27 => {
                        LightWallet::unsafe_from_buffer_regtest(include_bytes!(
                            "examples/regtest/hmvasmuvwmssvichcarbpoct/v27/zingo-wallet.dat"
                        ))
                        .await
                    }
                },
                RegtestSeedVersion::AbandonAbandon(version) => match version {
                    AbandonAbandonVersion::V26 => {
                        LightWallet::unsafe_from_buffer_regtest(include_bytes!(
                            "examples/regtest/aaaaaaaaaaaaaaaaaaaaaaaa/v26/zingo-wallet.dat"
                        ))
                        .await
                    }
                },
                RegtestSeedVersion::AbsurdAmount(version) => match version {
                    AbsurdAmountVersion::OrchAndSapl => {
                        LightWallet::unsafe_from_buffer_regtest(include_bytes!(
                        "examples/regtest/aadaalacaadaalacaadaalac/orch_and_sapl/zingo-wallet.dat"
                    ))
                        .await
                    }
                    AbsurdAmountVersion::OrchOnly => {
                        LightWallet::unsafe_from_buffer_regtest(include_bytes!(
                            "examples/regtest/aadaalacaadaalacaadaalac/orch_only/zingo-wallet.dat"
                        ))
                        .await
                    }
                },
            },
            NetworkSeedVersion::Testnet(seed) => match seed {
                TestnetSeedVersion::ChimneyBetter(version) => match version {
                    ChimneyBetterVersion::V26 => {
                        LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                            "examples/testnet/cbbhrwiilgbrababsshsmtpr/v26/zingo-wallet.dat"
                        ))
                        .await
                    }
                    ChimneyBetterVersion::V27 => {
                        LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                            "examples/testnet/cbbhrwiilgbrababsshsmtpr/v27/zingo-wallet.dat"
                        ))
                        .await
                    }
                    ChimneyBetterVersion::V28 => {
                        LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                            "examples/testnet/cbbhrwiilgbrababsshsmtpr/v28/zingo-wallet.dat"
                        ))
                        .await
                    }
                    ChimneyBetterVersion::Latest => {
                        LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                            "examples/testnet/cbbhrwiilgbrababsshsmtpr/latest/zingo-wallet.dat"
                        ))
                        .await
                    }
                },
                TestnetSeedVersion::MobileShuffle(version) => match version {
                    MobileShuffleVersion::Gab72a38b => {
                        LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                            "examples/testnet/mskmgdbhotbpetcjwcspgopp/Gab72a38b/zingo-wallet.dat"
                        ))
                        .await
                    }
                    MobileShuffleVersion::G93738061a => {
                        LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                            "examples/testnet/mskmgdbhotbpetcjwcspgopp/G93738061a/zingo-wallet.dat"
                        ))
                        .await
                    }
                    MobileShuffleVersion::Latest => {
                        LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                            "examples/testnet/mskmgdbhotbpetcjwcspgopp/latest/zingo-wallet.dat"
                        ))
                        .await
                    }
                },
                TestnetSeedVersion::GloryGoddess => {
                    LightWallet::unsafe_from_buffer_testnet(include_bytes!(
                        "examples/testnet/glory_goddess/latest/zingo-wallet.dat"
                    ))
                    .await
                }
            },
            NetworkSeedVersion::Mainnet(seed) => match seed {
                MainnetSeedVersion::VillageTarget(version) => match version {
                    VillageTargetVersion::V28 => {
                        LightWallet::unsafe_from_buffer_mainnet(include_bytes!(
                            "examples/mainnet/vtfcorfbcbpctcfupmegmwbp/v28/zingo-wallet.dat"
                        ))
                        .await
                    }
                },
                MainnetSeedVersion::HotelHumor(version) => match version {
                    HotelHumorVersion::Gf0aaf9347 => {
                        LightWallet::unsafe_from_buffer_mainnet(include_bytes!(
                            "examples/mainnet/hhcclaltpcckcsslpcnetblr/gf0aaf9347/zingo-wallet.dat"
                        ))
                        .await
                    }
                    HotelHumorVersion::Latest => {
                        LightWallet::unsafe_from_buffer_mainnet(include_bytes!(
                            "examples/mainnet/hhcclaltpcckcsslpcnetblr/latest/zingo-wallet.dat"
                        ))
                        .await
                    }
                },
            },
        }
    }
    /// picks the seed (or ufvk) string associated with an example wallet
    pub fn example_wallet_base(&self) -> String {
        match self {
            NetworkSeedVersion::Regtest(seed) => match seed {
                RegtestSeedVersion::HospitalMuseum(_) => {
                    testvectors::seeds::HOSPITAL_MUSEUM_SEED.to_string()
                },
                RegtestSeedVersion::AbandonAbandon(_) => {
                    testvectors::seeds::ABANDON_ART_SEED.to_string()
                },
                RegtestSeedVersion::AbsurdAmount(_) => {
                    "absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter advice comic".to_string()
                }
            },
            NetworkSeedVersion::Testnet(seed) => match seed {
                TestnetSeedVersion::ChimneyBetter(
                    _,
                ) => testvectors::seeds::CHIMNEY_BETTER_SEED.to_string(),
                TestnetSeedVersion::MobileShuffle(
                    _,
                ) => "mobile shuffle keen mother globe desk bless hub oil town begin potato explain table crawl just wild click spring pottery gasp often pill plug".to_string(),
                TestnetSeedVersion::GloryGoddess => "glory goddess cargo action guilt ball coral employ phone baby oxygen flavor solid climb situate frequent blade pet enough milk access try swift benefit".to_string(),
            },
            NetworkSeedVersion::Mainnet(seed) => match seed {
                MainnetSeedVersion::VillageTarget(
                     _,
                ) => "village target fun course orange release female brain cruise birth pet copy trouble common fitness unfold panther man enjoy genuine merry write bulb pledge".to_string(),
                MainnetSeedVersion::HotelHumor(
                     _,
                ) => "hotel humor crunch crack language awkward lunar term priority critic cushion keep coin sketch soap laugh pretty cement noodle enjoy trip bicycle list return".to_string()
            }
        }
    }
    /// picks the first receiver associated with an example wallet
    pub fn example_wallet_address(&self, pool: PoolType) -> String {
        match self {
            NetworkSeedVersion::Regtest(seed) => match seed {
                RegtestSeedVersion::HospitalMuseum(_) => {
                    match pool {
                        PoolType::Transparent => {"tmFLszfkjgim4zoUMAXpuohnFBAKy99rr2i".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"zregtestsapling1fkc26vpg566hgnx33n5uvgye4neuxt4358k68atnx78l5tg2dewdycesmr4m5pn56ffzsa7lyj6".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"uregtest1wdukkmv5p5n824e8ytnc3m6m77v9vwwl7hcpj0wangf6z23f9x0fnaen625dxgn8cgp67vzw6swuar6uwp3nqywfvvkuqrhdjffxjfg644uthqazrtxhrgwac0a6ujzgwp8y9cwthjeayq8r0q6786yugzzyt9vevxn7peujlw8kp3vf6d8p4fvvpd8qd5p7xt2uagelmtf3vl6w3u8".to_string()},
                    }
                },
                RegtestSeedVersion::AbandonAbandon(_) => {
                    match pool {
                        PoolType::Transparent => {"tmBsTi2xWTjUdEXnuTceL7fecEQKeWaPDJd".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"zregtestsapling1fmq2ufux3gm0v8qf7x585wj56le4wjfsqsj27zprjghntrerntggg507hxh2ydcdkn7sx8kya7p".to_string().to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"uregtest1zkuzfv5m3yhv2j4fmvq5rjurkxenxyq8r7h4daun2zkznrjaa8ra8asgdm8wwgwjvlwwrxx7347r8w0ee6dqyw4rufw4wg9djwcr6frzkezmdw6dud3wsm99eany5r8wgsctlxquu009nzd6hsme2tcsk0v3sgjvxa70er7h27z5epr67p5q767s2z5gt88paru56mxpm6pwz0cu35m".to_string()},
                    }
                },
                RegtestSeedVersion::AbsurdAmount(_) => {
                    match pool {
                        PoolType::Transparent => {"tmS9nbexug7uT8x1cMTLP1ABEyKXpMjR5F1".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"zregtestsapling1lhjvuj4s3ghhccnjaefdzuwp3h3mfluz6tm8h0dsq2ym3f77zsv0wrrszpmaqlezm3kt6ajdvlw".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"uregtest1qtqr46fwkhmdn336uuyvvxyrv0l7trgc0z9clpryx6vtladnpyt4wvq99p59f4rcyuvpmmd0hm4k5vv6j8edj6n8ltk45sdkptlk7rtzlm4uup4laq8ka8vtxzqemj3yhk6hqhuypupzryhv66w65lah9ms03xa8nref7gux2zzhjnfanxnnrnwscmz6szv2ghrurhu3jsqdx25y2yh".to_string()},
                    }
                },
            },
            NetworkSeedVersion::Testnet(seed) => match seed {
                TestnetSeedVersion::ChimneyBetter(_) => {
                    match pool {
                        PoolType::Transparent => {"tmYd5GP6JxUxTUcz98NLPumEotvaMPaXytz".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"ztestsapling1etnl5s47cqves0g5hk2dx5824rme4xv4aeauwzp4d6ys3qxykt5sw5rnaqh9syxry8vgxu60uhj".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"utest17wwv8nuvdnpjsxtu6ndz6grys5x8wphcwtzmg75wkx607c7cue9qz5kfraqzc7k9dfscmylazj4nkwazjj26s9rhyjxm0dcqm837ykgh2suv0at9eegndh3kvtfjwp3hhhcgk55y9d2ys56zkw8aaamcrv9cy0alj0ndvd0wll4gxhrk9y4yy9q9yg8yssrencl63uznqnkv7mk3w05".to_string()},
                    }
                },
                TestnetSeedVersion::MobileShuffle(_) => {
                    match pool {
                        PoolType::Transparent => {"tmEVmDAnveCakZkvV4a6FT1TfYApTv937E7".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"ztestsapling1h8l5mzlwhmqmd9x7ehquayqckzg6jwa6955f3w9mnkn5p5yfhqy04yz6yjrqfcztxx05xlh3prq".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"utest19zd9laj93deq4lkay48xcfyh0tjec786x6yrng38fp6zusgm0c84h3el99fngh8eks4kxv020r2h2njku6pf69anpqmjq5c3suzcjtlyhvpse0aqje09la48xk6a2cnm822s2yhuzfr47pp4dla9rakdk90g0cee070z57d3trqk87wwj4swz6uf6ts6p5z6lep3xyvueuvt7392tww".to_string()},
                    }
                },
                TestnetSeedVersion::GloryGoddess => {
                    match pool {
                        PoolType::Transparent => {"tmF7QpuKsLF7nsMvThu4wQBpiKVGJXGCSJF".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"ztestsapling17d9jswjas4zt852el3au4fpnvelc5k76d05fga2rdm9zttac29m8g202dkz8wsfcd7r4c8q540n".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"utest1dfj7pz3v2hewxk5nt36h9mu9g5h9cuc3064h5jlzgnq9f9f74fv4vr4vz4yvp2mq36ave8y8ggghm4g0e2nw9fg7qewuflpxxtvq4dzpscws4eq0hlh09shhj83a5yuzm9p3hsrtd028xk37gyf6403g90jwvtxkgyqv25kzzafyjfl7rvexzvrjx24akdr83qzkssyg22jm5cgcxc9".to_string()},
                    }
                },
            },
            NetworkSeedVersion::Mainnet(seed) => match seed {
                MainnetSeedVersion::VillageTarget(_) => {
                    match pool {
                        PoolType::Transparent => {
                    "t1P8tQtYFLR7TWsqtauc71RGQdqqwfFBbb4".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"zs1kgdrzfe6xuq3tg64vnezp3duyp43u7wcpgduqcpwz9wsnfqm4cecafu9qkmpsjtqxzf27n34z9k".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"u1n5zgv8c9px4hfmq7cr9f9t0av6q9nj5dwca9w0z9jxegut65gxs2y4qnx7ppng6k2hyt0asyycqrywalzyasxu2302xt4spfqnkh25nevr3h9exc3clh9tfpr5hyhc9dwee50l0cxm7ajun5xs9ycqhlw8rd39jql8z5zlv9hw4q8azcgpv04dez5547geuvyh8pfzezpw52cg2qknm".to_string()},
                    }
                },
                MainnetSeedVersion::HotelHumor(_) => {
                    match pool {
                        PoolType::Transparent => {"t1XnsupYhvhSDSFJ4nzZ2kADhLMR22wg35y".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Sapling) => {"zs1zgffhwsnh7efu4auv8ql9egteangyferp28rv8r7hmu76u0ee8mthcpflx575emx2zygqcuedzn".to_string()},
                        PoolType::Shielded(ShieldedProtocol::Orchard) => {"u14lrpa0myuh5ax8dtyaj64jddk8m80nk2wgd3sjlu7g3ejwxs3qkfj5hntakjte8ena3qnk40ht0ats5ad0lcwhjtn9hak6733fdf33fhkl7umgqy2vtcfmhmca9pjdlrsz68euuw06swnl9uzzpadmvztd50xen4ruw738t995x7mhdcx3mjv7eh5hntgtvhtv6vgp9l885eqg6xpm8".to_string()},
                    }
                },
            },
        }
    }
}
