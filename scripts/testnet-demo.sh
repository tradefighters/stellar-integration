#!/usr/bin/env bash
# Runs three demo duels against a deployed tf-competition on Stellar testnet
# and prints a markdown table of the transactions with stellar.expert links.
#
#   duel 1: create -> join -> settle Win(player A)
#   duel 2: create -> join -> settle Draw
#   duel 3: create -> cancel by the creator
#
# Needs these stellar-cli identities, funded and holding testnet USDC
# (see docs/SOROBAN-SETUP.md § 5): tf-resolver, tf-player-a, tf-player-b.
#
# Usage: scripts/testnet-demo.sh <contract-id> [stake in USDC base units]
set -euo pipefail

CONTRACT="${1:?usage: $0 <contract-id> [stake]}"
STAKE="${2:-10000000}"   # 1 USDC (7 decimals)
SCC=1000000000           # $1 000 virtual starting capital (6 decimals)
DELAY=90                 # seconds until start (contract minimum is 60)
WINDOW=60                # trading window in seconds
NET=testnet
EXPLORER=https://stellar.expert/explorer/testnet/tx

A=$(stellar keys address tf-player-a)
B=$(stellar keys address tf-player-b)
ROWS_FILE=$(mktemp)
trap 'rm -f "$ROWS_FILE"' EXIT

# invoke <label> <identity> <fn> [args...]: runs the call, records the tx hash,
# prints the return value on stdout.
invoke() {
  local label="$1" who="$2" fn="$3"
  shift 3
  local out
  out=$(stellar contract invoke --id "$CONTRACT" --network "$NET" \
    --source-account "$who" -- "$fn" "$@" 2>&1)
  local hash
  hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' <<<"$out" | tail -1 | awk '{print $3}')
  if [[ -z "$hash" ]]; then
    echo "call failed: $label" >&2
    echo "$out" >&2
    exit 1
  fi
  # A file, not an array: create() runs in a subshell.
  echo "| $label | [\`${hash:0:12}…\`]($EXPLORER/$hash) |" >>"$ROWS_FILE"
  # The return value is the last line that isn't a CLI log line (none for
  # calls that return nothing).
  grep -vE '^(ℹ️|🌎|✅|🔗|📅)' <<<"$out" | tail -1 || true
}

create() {
  local now starts ends
  now=$(date +%s)
  starts=$((now + DELAY))
  ends=$((starts + WINDOW))
  invoke "duel $1: create_duel (player A, stake $STAKE)" tf-player-a create_duel \
    --creator "$A" \
    --params "{\"stake\":\"$STAKE\",\"scc\":$SCC,\"starts_at\":$starts,\"ends_at\":$ends}" |
    tr -d '"'
}

echo "Creating and joining duels 1 and 2, creating duel 3…" >&2
D1=$(create 1)
invoke "duel $D1: join_duel (player B)" tf-player-b join_duel --duel_id "$D1" --player "$B" >/dev/null
D2=$(create 2)
invoke "duel $D2: join_duel (player B)" tf-player-b join_duel --duel_id "$D2" --player "$B" >/dev/null
D3=$(create 3)
invoke "duel $D3: cancel_duel (creator, refund)" tf-player-a cancel_duel --duel_id "$D3" --caller "$A" >/dev/null

# Wait until both active duels have ended (+ margin for ledger close time).
WAIT=$((DELAY + WINDOW + 15))
echo "Waiting ${WAIT}s for the trading window to end…" >&2
sleep "$WAIT"

invoke "duel $D1: settle_duel Win(player A)" tf-resolver settle_duel \
  --duel_id "$D1" --outcome "{\"Win\":\"$A\"}" >/dev/null
invoke "duel $D2: settle_duel Draw" tf-resolver settle_duel \
  --duel_id "$D2" --outcome '"Draw"' >/dev/null

echo
echo "| Step | Transaction |"
echo "| --- | --- |"
cat "$ROWS_FILE"
