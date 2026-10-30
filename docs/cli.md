# CLI Interface

The `edw-cli` crate implements the `edw` CLI interface.
The crate is concerned purely with inputs and outputs, not with underlying machinery.

## Layout

```sh
edw lock/unlock # locks or unlocks the datastore
edw config path # prints the data directory
edw config view # the data directory, its network instances, the session, and the active RPC config; works while locked

edw network view # the active RPC, the bundler URLs, and the indexer
edw network rpc-config list # lists this network's RPC configs; the active one is marked
edw network rpc-config add <name> --url <url> [--blockrange 500] [--interval 0ms] [--use] # adds an RPC config. Active when none is, or when --use is set. --blockrange is blocks per eth_getLogs request. --interval is the pause between calls; the seeded publicnode config uses 500ms
edw network rpc-config use <name> # makes an RPC config the active one
edw network bundler-config list # bundler URLs in fallback order
edw network bundler-config add --url <url> # appends a fallback. The list always keeps at least one URL once it has one
edw network bundler-config remove --url <url> # removes a URL. Refuses to remove the last one
edw network indexer-config [--url <url>] [--enable | --disable] # shows the indexer, replaces its URL, or turns it on or off

edw profile list # lists all configured profiles
edw profile new # creates a new profile; on a new phrase, with an address vault for its first address, else like import
edw profile import # imports a profile from a mnemonic, with an address vault for each address up to the last used one (or the first, if none is used or the scan fails), and scans the chain's Tornado pools (stops after 20 consecutive unused nonces)
edw profile rescan [profile] # scans a profile's addresses again and adds an address vault for each one it is missing, and scans the chain's Tornado pools the same way
edw profile clear-pending [profile] [--nonce N] # syncs tracked pools, then forgets unmined deposit reservations without reusing nonces
edw profile reveal-phrase # shows a profile's recovery phrase, at a terminal only

edw asset list [--profile SELECTOR] # the ERC-20s a profile tracks: the network's defaults, minus opt-outs, then its opt-ins
edw asset opt-in <address> [--profile SELECTOR] # tracks an ERC-20, or restores a default the profile opted out of
edw asset opt-out <address> [--profile SELECTOR] # stops tracking an ERC-20. A default is recorded as opted out; any other token is removed from its opt-ins. That hides its public balance and its Tornado notes
edw balance [profile] [--detailed] # each asset's public, private and total holdings; the private total includes Tornado notes. --detailed lists each vault and each unspent Tornado note
edw next-fresh-address [--peek] [--profile SELECTOR]
edw shield [--amount-formatted <tokens> | --amount-raw <base-units>] [--token SYMBOL] [--denominations <sizes,...>] [--prepare | --from INDEX] [--profile SELECTOR] [--yes]
edw unshield [--amount-formatted <tokens> | --amount-raw <base-units>] [--token SYMBOL] [--to <address> | --tail-calls <target:calldata[:wei],...> [--sweep-to ADDRESS]] [--quote] [--expected-fresh-address ADDRESS] [--profile SELECTOR] [--yes]
edw transfer --from <index|address|tornado> --to <address> --amount <eth> [--profile SELECTOR]
```

A command that finds the wallet locked unlocks it with `EDW_DECRYPTION_PASSWORD`, else a terminal prompt. A prompted password keeps a session, as `edw unlock` does, until 15 minutes after its last use; `EDW_DECRYPTION_PASSWORD` serves that one command. Piped stdin never holds a command's password, only its fields; a script sets `EDW_DECRYPTION_PASSWORD`, or, where a session can be kept, pipes the password to `edw unlock` first. Without a terminal, a command cannot choose between several network instances, so with more than one, run `edw unlock --network <name or id>` first. `edw lock` ends the session.

The session, password included, is a plaintext file under `$XDG_RUNTIME_DIR/edw`, which the system clears at logout. Without that variable, as on macOS, no session is kept: each command asks for the password again, or reads `EDW_DECRYPTION_PASSWORD`. A script there can only open a data directory that holds one network instance.

`shield` and `unshield` support ETH and the ERC-20 tokens below. `transfer` moves native ETH. At a terminal they ask for anything left off the command line, then wait for an explicit yes before broadcasting. `--non-interactive` asks for nothing; broadcasting also requires `--yes`. A fee quote and `shield --prepare` never broadcast and do not need `--yes`.

`shield` and `unshield` take the amount as `--amount-formatted` (decimal tokens, such as `1.1`) or `--amount-raw` (integer base units). Neither flag is required: a terminal prompts for the selected token amount. The amount has to be an exact sum of enabled Tornado denominations. `shield --denominations` chooses the exact ordered pool sizes; the total must match any supplied amount. A shield supports at most five notes. Extended ETH pools are enabled by default and can be disabled per profile. Unshield also requires the profile to hold unspent notes that sum to that amount.

`unshield` and `transfer --from tornado` submit one ERC-4337 user operation from the next fresh address. For ETH withdrawals, the largest selected note pays gas through the Tornado paymaster. That note's remainder, and every other note, land on the fresh address. Without `--to`, `unshield` leaves the remainder there. `unshield --to` and `transfer --from tornado` add a final call that forwards the remainder after the sponsor fee. A tornado transfer whose notes cannot cover `--amount` after that fee is an error. Every ERC-20 withdrawal additionally spends the smallest mined ETH note with a paymaster held by the profile on the same chain, even when the token pool has its own paymaster/adapter. Only the ETH note sponsors gas: token paymasters and adapters are never used. The full requested token amount reaches the fresh account, and the ETH sponsor's remainder stays there after fees. Without an eligible ETH note the command fails before proving. ETH withdrawals still require a selected note with a paymaster. Mainnet and Sepolia start on `https://public.pimlico.io/v2/{chainId}/rpc`. Further URLs added with `bundler-config add` are fallbacks, tried in order. A failed estimate or send tries the next URL. The receipt is read from the bundler that accepted the operation. A `shield` of one ETH note stays an ordinary transaction from the paying account. A `shield` of several notes, or any ERC-20 approval plus deposit, is one user operation from that account, sent to the same bundler. The operation installs the EIP-7702 delegation when the account does not already have it, and the account pays gas from its own ETH. Any other batch of calls from an EOA uses that same user operation. A transfer from an index or address stays an ordinary transaction.

`--token` is case insensitive and defaults to ETH. Mainnet supports USDC and USDT (6 decimals; 100 and 1,000 token pools), DAI (18 decimals; 100, 1,000, 10,000 and 100,000 pools), and WBTC (8 decimals; 0.1, 1 and 10 pools). Sepolia supports DAI (18 decimals; the 100 DAI pool). Token addresses come from the chain-specific Kohaku pool catalog; unsupported symbol/network pairs are rejected. Raw token amounts are never wei: for example, 100 USDC is `--amount-raw 100000000`. Fractional base units are rejected. Shielding checks both the token balance and ETH for gas, and batches approvals with deposits atomically.

`unshield --tail-calls` accepts comma-separated `TARGET:0xCALLDATA` or `TARGET:0xCALLDATA:VALUE` entries. Calldata must be byte-aligned hex; `0x` means empty calldata. `VALUE` is optional, defaults to zero, and is decimal wei or `0x` hex wei. Calls retain their supplied order, run from the fresh EIP-7702 account after all note withdrawals, and are included in the same UserOp. Use an approval call before a swap call when a router needs token allowance. `--to` and `--tail-calls` are mutually exclusive. For ETH, `--sweep-to ADDRESS` is shorthand for a final ETH transfer tail call. The wallet fills in its value from the quoted remainder after the preceding calls and maximum sponsor fee, then requotes the complete operation. An explicit transfer tail call works too when its value is known. The paymaster refund arrives after the UserOp calls and cannot be included in this sweep. ERC-20 withdrawals reject tail calls with nonzero ETH value.

`unshield --quote` performs the same fee estimation without signing, sending, or committing the fresh address. In `--non-interactive` mode it emits `edw/unshield-quote` JSON with `fee_raw` in wei and `available_raw` in the selected token's base units. `tail_value_raw` is the total ETH value budget for the supplied tail calls (zero for ERC-20); `sweep_value_raw` is the computed value of the final sweep call when `--sweep-to` is used. For one ETH consuming tail call, set its value below `tail_value_raw`, then quote again with that exact call before broadcasting; the changed calldata or gas market may change the fee. A quote is an estimate at that moment, so the final proven operation is checked again before signing.

For example (replace target addresses and calldata with the intended calls):

```sh
edw shield --token USDC --amount-formatted 100 --from 0
edw unshield --token DAI --amount-formatted 100 --to 0x1111111111111111111111111111111111111111
edw unshield --amount-formatted 1 --tail-calls '0x1111111111111111111111111111111111111111:0x:950000000000000000'
```

Gas is measured by the configured bundlers, with 20% headroom. During fee quoting, a simulation-only native ETH balance override lets calls consume the sponsoring note's proceeds. ERC-20 proceeds come from actual token withdrawals in the simulated execution. The proven operation is estimated again with **no funding override**; estimates exceeding the quoted fee abort before signing. The RPC and bundler must support state overrides; there is no guessed gas fallback. Native tail values must fit after the fee; token transfers and approvals are checked by final estimation.

Fee quoting uses a dummy UserOp signature; the fresh account key signs only during submission. CLI profiles derive software keys from mnemonics. Reports identify the withdrawal token, the fee token, and the number of custom tail calls separately. The reported fee is the proven maximum reserve; a paymaster refund of unused gas can leave additional change on the fresh account.

`shield --prepare --non-interactive` reserves new Tornado notes in the encrypted wallet database and returns one `edw/shield-prepared` JSON document. It never broadcasts, checks payer balances, or signs a transaction/UserOp, and needs no `--from`. `--prepare` and `--from` conflict. It constructs calls rather than independently simulating them: the final combined UserOp is simulated by `unshield` after its proceeds are available.

The document contains `chain_id`, `token`, `broadcast: false`, public `notes` metadata (nonce, pool ID, commitment, amount), ordered `calls` (`target`, `data`, decimal native-wei `value` strings), and `tail_calls`, a string ready for `unshield --tail-calls`. ERC-20 preparations include approval before each deposit. It contains no note secrets. To retry the same intended deposit, reuse the saved output; invoking preparation again deliberately creates different notes.

Reservation holds a cross-process database lock across nonce allocation and note persistence. The nonce advance is flushed to disk before derivation, and the pending record is saved before any calldata is returned. Failure, cancellation, abandoned calls, or a broken stdout pipe never roll back consumed nonces. Restarting continues from that high-water mark. Rescan preserves local pending reservations and never rewinds the counter. Exported calls remain valid after a failed simulation, so do not recycle their nonces or roll the wallet database back to an older backup while those calls might still be submitted. Keep the encrypted wallet database backed up with the recovery phrase.

A seed-only restore cannot reconstruct reservations that never appeared on-chain. Import/rescan normally discover notes until 20 consecutive nonces have no deposits. After many abandoned preparations, use `profile import --tornado-gap-limit N` or `profile rescan PROFILE --tornado-gap-limit N` with a gap large enough to cross the unused range and discover later mined notes. This recovers on-chain notes; it does not recover unbroadcast reservations or make an old database safe to reuse alongside newer exported calls.

For an agent composing ETH unshield → swap → USDC shield (with wallet credentials/session already configured):

```sh
edw next-fresh-address --peek --profile alice --non-interactive
edw shield --prepare --token USDC --amount-formatted 100 --profile alice --non-interactive
# Save the preparation JSON. Append its tail_calls after the swap calls.
edw unshield --amount-formatted 1 --profile alice --non-interactive --yes \
  --expected-fresh-address "$FRESH_ADDRESS" \
  --tail-calls "$SWAP_CALLS,$SHIELD_CALLS"
```

`unshield --non-interactive --yes` broadcasts after proving and simulation. There is no `--broadcast` flag and no `--next` flag: unshield already uses the next fresh account. To inspect fees without submission, use `unshield --quote`; to construct deposit calls, use `shield --prepare`. A swap should guarantee at least the chosen pool denominations, with its output available to the executing fresh account. Any surplus remains there.

`--expected-fresh-address` is optional. If supplied, unshield aborts before proving/signing/submission when its selected account differs. A mismatch does not inherently guarantee an EVM revert: arbitrary calldata can still succeed with a recipient address baked into an earlier quote, so use the explicit check when calls were built for a peeked address.

`next-fresh-address` prints the next standard address that this profile has not handed out and that has no nonce, code, or ETH, then stores it. `--peek` prints the same address and writes nothing. A declined command and `unshield --quote` do not store a fresh address. `unshield` and `transfer --from tornado` store it before submission, then wait for the receipt.

Pool sync tries the Saga indexer when it is on (`https://saga.gordosoluciones.xyz` on mainnet, `https://saga.gordosoluciones.xyz/sepolia` on Sepolia) and finishes any remaining blocks with `eth_getLogs` on the active RPC. Each log request covers that RPC config's block range (500 by default), and the config's call interval is the pause between RPC requests. Other chains start with the indexer off and use the RPC only. `indexer-config --disable` turns it off and keeps the URL. Import, rescan, and balance read this profile's enabled Tornado pools on the chain. Besides the ETH pools, mainnet includes DAI (100, 1,000, 10,000, 100,000), USDC (100, 1,000), USDT (100, 1,000), and WBTC (0.1, 1, 10); Sepolia includes DAI 100. USDC, USDT, DAI, and WBTC are defaults on mainnet, and DAI is a default on Sepolia. Every address vault is read for them unless the profile opts out.

### Global Args

```sh
--non-interactive, --porcelain # runs in non-interactive mode
--data-dir # runs with the specified data directory; env: EDW_DATA_DIR
-v, --verbose # logs more to stderr, repeatable: info, debug, trace
-q, --quiet # logs less to stderr, repeatable: errors only, then nothing
```

The data directory defaults to `~/.edw`, or `./.edw/` when there is no home directory. Earlier versions defaulted to `./.edw/` and read `DATA_DIR`; move that directory to `~/.edw`, or pass `--data-dir ./.edw`.

Logs go to stderr at `warn` by default. `RUST_LOG`, when set, takes precedence over `-v` and `-q`.

### Exit Codes

| Code | Class | When |
|---|---|---|
| 0 | | the command succeeded, or a confirmation was declined |
| 1 | `other` | any failure not listed below, including a value the wallet core rejects, such as an unknown profile name or an amount that is not a sum of pool denominations, an unreachable RPC endpoint or bundler, or a reverted transaction |
| 2 | `usage` | an argument the CLI rejects itself, such as a missing flag, a malformed address, or an unusable phrase file |
| 3 | `locked` | no session or password to open the wallet with, or a wrong password |

With `--non-interactive`, a failure prints one `edw/error` document on stdout instead of text on stderr. Argument errors that clap reports while parsing are always text, with code 2.

```json
{"kind":"edw/error","version":1,"class":"locked","message":"wallet is locked; run `edw unlock` or set EDW_DECRYPTION_PASSWORD"}
```
