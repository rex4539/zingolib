SUCCESSES=0
for i in {0..15};
do
  if $(cargo nextest run --run-ignored=all mainnet_latest_send_to_self_orchard_hot);
  then
    SUCCESSES++
  fi
done
echo $SUCCESSES
