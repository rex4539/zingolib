(optional) to update the hot wallet in order to minimize execution time:

cargo run -- --data-dir zingolib/src/wallet/disk/testing/examples/mainnet/hhcclaltpcckcsslpcnetblr/latest/


to run a basic hot test:

cargo nextest run --run-ignored=all mainnet_latest_send_to_self_orchard_hot



to run 16 tests, in order to check for flakiness:
$G/zingolib/bash/repeated_test.sh
