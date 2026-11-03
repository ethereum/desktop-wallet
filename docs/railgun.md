# Railgun in EDW

EDW uses the native `kohaku-railgun`, `kohaku-railgun-sync`,
`kohaku-railgun-poi` and `kohaku-railgun-paymaster` crates at kohaku-rs revision
`357104d3be2e06d91f532563f09c5a9804be088e` (`feat/separate-crates`).
Ethereum mainnet and Sepolia are supported. No JavaScript wrapper is needed.
The circuits themselves still use their Circom WASM witness calculators inside
the native prover, as the Rust SDK does.

## Commands

Tornado remains the default for `shield` and `unshield`; select Railgun explicitly:

```sh
edw railgun address --profile 0/0
edw shield --protocol railgun --profile 0/0 --from 0 --token ETH --amount-formatted 0.1
edw shield --protocol railgun --profile 0/0 --from 0 --token WETH --amount-formatted 0.01
edw balance 0/0 --detailed
edw unshield --protocol railgun --profile 0/0 --token ETH --amount-formatted 0.01 --quote
edw unshield --protocol railgun --profile 0/0 --token USDC --amount-formatted 10 --to 0xRECIPIENT
edw transfer --from railgun --profile 0/0 --asset WETH --amount 0.01 --to 0zkRECIPIENT
edw transfer --from railgun --profile 0/0 --asset ETH --amount 0.01 --to 0xRECIPIENT
```

`--token` is an alias for transfer's `--asset`. Transfer supports `--dry-run`
and an encrypted `--memo` for 0zk recipients. A 0zk recipient receives WETH when
ETH is selected; internal transfers remain shielded. Address checks reject an
advisory chain ID that differs from the selected network. Other ERC-20s must be
tracked with `edw asset opt-in <address>` and can be selected by symbol or address.
Amounts are variable; `--denominations` only applies to Tornado.

Shielding spends from the selected public account. Native ETH is wrapped and
shielded via RelayAdapt. ERC-20 approval and shield are sequential public
transactions, each mined before the next one; their gas is paid in public ETH.
`shield --prepare` returns public call tuples without broadcasting.

Private spends use one ERC-4337 UserOperation with a fresh profile account and
its 7702 authorization. **The sponsor fee is always paid from private WETH**,
including USDC and other token unshields and internal transfers. Fund private
WETH first. The quote fixes gas using dummy proofs before asking for a spending
signature; the real proof is checked against those limits before submission.
There is no fallback to spending the selected token for gas.

An unshield amount is the exact net amount received by the fresh account.
The protocol fee is added to the private input amount. WETH is unshielded and
unwrapped before forwarding ETH or executing native tail calls. WETH balances
are shown as WETH; they are never counted again as native ETH.

`unshield --tail-calls TARGET:CALLDATA[:WEI],...` executes the calls in order
from the fresh account after unshielding (and after the native unwrap). Native
call values must fit within an ETH unshield. Token unshields support token calls
with zero ETH value. `--sweep-to` appends an ETH transfer of the amount remaining
after custom call values; the sponsor fee is paid separately from private WETH.
`--expected-fresh-address` binds the command to a previously inspected address.
`--quote` does not sign a spend, submit PPOI proofs, send an operation or consume
a fresh address. Normal broadcasts require confirmation, or `--yes`.

## PPOI and recovery

```sh
edw railgun sync --profile 0/0
edw railgun sync --profile 0/0 --observe-only
edw railgun retry --profile 0/0
```

Synchronization copies and validates PPOI lists, indexes TXIDs, and recovers
historical missing proofs. Normal sync processes the pending proof queue;
`--observe-only` leaves it local. Submission is an attempt, not proof acceptance.
A note becomes spendable only when the validated local list reports `Valid`.
Missing, unavailable and submitted-but-unaccepted notes are included in pending
balances and retain their explicit PPOI status in detailed output. Waiting or
failed proof processing is reported and can be retried with `railgun sync`.

UTXO, TXID, PPOI and pending-operation data share one versioned encrypted
checkpoint in the network/profile's `railgun` scope. Every completed sync window
is saved atomically. Keys are derived again from the mnemonic and are not saved
in checkpoints. Baby Jubjub derivation matches kohaku-cli and the Railgun engine,
with the profile index in the final hardened path component.

The signed operation and its pending PPOI inputs are persisted **before** sending.
An uncertain send blocks further private spending from that profile.
`railgun retry` resends the exact signed operation, or finds its existing receipt,
then processes PPOI. It never constructs a second spend. Keep the pending journal
when a bundler times out; a timeout does not establish transaction failure.

## Saga and RPC

EDW consumes Saga's `railgun-1-smartwallet` and `railgun-1-ops` streams from the
configured network indexer URL. It verifies gzip chunk SHA-256 digests, record
ordering and contiguous coverage, decodes all historical event signatures, and
passes the rebuilt UTXO/TXID trees through the SDK's validation. The operations
stream omits `hasUnshield`; EDW derives it by matching the final commitment
against that block's emitted UTXO leaves.

RPC covers the uncovered tip, including calldata nested in RelayAdapt or in
EntryPoint paymaster data. Calldata candidates must match emitted nullifiers and
output leaves, and operations are ordered by their chain log positions. Unknown
or incomplete calldata fails without advancing the TXID checkpoint. Direct
Subsquid reads are not used.

Saga currently publishes Railgun on mainnet. Sepolia uses RPC. Mainnet's legacy
operation era needs Saga coverage; if that history is unavailable, EDW fails
rather than inventing TXID metadata. PPOI list/root checks, TXID validation,
proof submission and live chain checks still contact their configured services.
First sync can take time, especially PPOI list catch-up; subsequent runs resume
from encrypted checkpoints.

Circuit artifacts use the SDK's pinned remote source. An alternate compatible
server can be selected with `--railgun-artifact-url` or
`EDW_RAILGUN_ARTIFACT_URL`.

## Validation

Local tests cover mnemonic parity, historical Saga event decoding, coverage
holes, ERC-4337 calldata reconstruction, exact net amounts, unwrap/tail order,
PPOI balance classification and interrupted-send persistence. The small historical
fixture contains public on-chain events from the first Saga smartwallet chunk;
its upstream format is documented in
[the Saga Railgun guide](https://github.com/fatlabsxyz/saga-sync/blob/master/docs/RAILGUN.md).

To check an entire public chunk without live RPC/PPOI traffic, download
`railgun-1-smartwallet-[0xe03295,0xfea77b).jsonl.gz` from the configured Saga
publication as `edw-saga-logs.gz`, then run:

```sh
EDW_RAILGUN_SAGA_FIXTURE_DIR=/path/to/chunks cargo test --manifest-path crates/Cargo.toml full_public_chunk_decoding -- --ignored
```
