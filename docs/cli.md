# CLI Interface

The `edw-cli` crate implements the `edw` CLI interface.
The crate is concerned purely with inputs and outputs, not with underlying machinery.

## Layout

A command that finds the wallet locked unlocks it with `EDW_DECRYPTION_PASSWORD`, else a terminal prompt. A prompted password keeps a session, as `edw unlock` does, until 15 minutes after its last use; `EDW_DECRYPTION_PASSWORD` serves that one command. Piped stdin never holds a command's password, only its fields; a script sets `EDW_DECRYPTION_PASSWORD`, or, where a session can be kept, pipes the password to `edw unlock` first. Without a terminal, a command cannot choose between several network instances, so with more than one, run `edw unlock --network <name or id>` first. `edw lock` ends the session.

The session, password included, is a plaintext file under `$XDG_RUNTIME_DIR/edw`, which the system clears at logout. Without that variable, as on macOS, no session is kept: each command asks for the password again, or reads `EDW_DECRYPTION_PASSWORD`. A script there can only open a data directory that holds one network instance.

`shield`, `unshield`, and `transfer` move native ETH. At a terminal they ask for anything left off the command line, then wait for an explicit yes before broadcasting. `--non-interactive` asks for nothing: a missing required field is an error, and invoking the command is the authorization. Piped stdin supplies the missing fields and skips the confirmation.

`shield` and `unshield` take the amount as `--amount-formatted` (decimal ETH, such as `1.1`) or `--amount-raw` (integer wei). Neither flag is required: a terminal prompts for decimal ETH, and piped stdin supplies that same line. The amount has to be an exact sum of the chain's Tornado denominations (mainnet `0.1`, `1`, `10`, `100` ETH; Sepolia `0.1`, `1`, `10` ETH). `1.15` ETH and `1` wei are rejected. Unshield also requires the profile to hold unspent notes that sum to that amount.

`unshield` and `transfer --from tornado` submit one ERC-4337 user operation from the next fresh address. The largest note pays gas through the Tornado paymaster. That note's remainder, and every other note, land on the fresh address. Without `--to`, `unshield` leaves the remainder there. `unshield --to` and `transfer --from tornado` add a final call that forwards the remainder after the sponsor fee. A tornado transfer whose notes cannot cover `--amount` after that fee is an error. A pool with no paymaster is an error. Mainnet and Sepolia start on `https://public.pimlico.io/v2/{chainId}/rpc`. Further URLs added with `bundler-config add` are fallbacks, tried in order. A failed estimate or send tries the next URL. The receipt is read from the bundler that accepted the operation. A `shield` of one note stays an ordinary transaction from the paying account. A `shield` of several notes is one user operation from that account, sent to the same bundler. The operation installs the EIP-7702 delegation when the account does not already have it, and the account pays gas from its own ETH. Any other batch of calls from an EOA uses that same user operation. A transfer from an index or address stays an ordinary transaction.

`next-fresh-address` prints the next standard address that this profile has not handed out and that has no nonce, code, or ETH, then stores it. `--peek` prints the same address and writes nothing. A declined command does not store a fresh address. `unshield` and `transfer --from tornado` store it once a bundler accepts the user operation, then wait for the receipt.

Pool sync tries the Saga indexer when it is on (`https://saga.gordosoluciones.xyz` on mainnet, `https://saga.gordosoluciones.xyz/sepolia` on Sepolia) and finishes any remaining blocks with `eth_getLogs` on the active RPC. Each log request covers that RPC config's block range (500 by default), and the config's call interval is the pause between RPC requests. Other chains start with the indexer off and use the RPC only. `indexer-config --disable` turns it off and keeps the URL. Import, rescan, and balance read every Tornado pool on the chain. Besides the ETH pools, mainnet includes DAI (100, 1,000, 10,000, 100,000), USDC (100, 1,000), USDT (100, 1,000), and WBTC (0.1, 1, 10); Sepolia includes DAI 100. USDC, USDT, DAI, and WBTC are defaults on mainnet, and DAI is a default on Sepolia. Every address vault is read for them unless the profile opts out.

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
