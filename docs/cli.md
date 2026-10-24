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
edw profile reveal-phrase # shows a profile's recovery phrase, at a terminal only

edw asset list [--profile SELECTOR] # the ERC-20s a profile tracks: the network's defaults, minus opt-outs, then its opt-ins
edw asset opt-in <address> [--profile SELECTOR] # tracks an ERC-20, or restores a default the profile opted out of
edw asset opt-out <address> [--profile SELECTOR] # stops tracking an ERC-20. A default is recorded as opted out; any other token is removed from its opt-ins. That hides its public balance and its Tornado notes
edw balance [profile] [--detailed] # each asset's public, private and total holdings; the private total includes Tornado notes. --detailed lists each vault and each unspent Tornado note
edw next-fresh-address [--peek] [--profile SELECTOR]
edw shield [--amount-formatted <eth> | --amount-raw <wei>] [--from INDEX] [--profile SELECTOR]
edw unshield [--amount-formatted <eth> | --amount-raw <wei>] [--to <address>] [--profile SELECTOR]
edw transfer --from <index|address|tornado> --to <address> --amount <eth> [--profile SELECTOR]
```

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
| 1 | `other` | any failure not listed below, such as a reverted transaction |
| 2 | `usage` | an invalid or missing argument or value |
| 3 | `locked` | no session or password to open the wallet with, or a wrong password |
| 4 | `not_found` | no wallet instance, profile, RPC config, account, or tracked asset matches |
| 5 | `network` | an RPC endpoint could not be reached, rate-limited the request, or serves the wrong chain, or a bundler failed |

With `--non-interactive`, a failure prints one `edw/error` document on stdout instead of text on stderr. Argument errors that clap reports while parsing are always text, with code 2.

```json
{"kind":"edw/error","version":1,"class":"locked","message":"wallet is locked; run `edw unlock`"}
```
