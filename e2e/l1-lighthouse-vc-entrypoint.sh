#!/bin/bash
set -exu

/wait && echo "Lighthouse Validator Client is ready"

# Validator keys and secrets are directly mounted
# Check if they are properly mounted
echo "Setting up validator keys..."
ls -la /db/validators/ | head -5
ls -la /db/secrets/ | head -5
echo "Validator keys are directly mounted and ready"

exec /usr/local/bin/lighthouse \
  vc \
  --datadir="/db" \
  --beacon-nodes="${LH_BEACON_NODES}" \
  --testnet-dir=/genesis \
  --init-slashing-protection \
  --suggested-fee-recipient="0xff00000000000000000000000000000000c0ffee" \
  "$@"
