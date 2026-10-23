# CLI Interface

The `edw-cli` crate implements the `edw` CLI interface.
The crate is concerned purely with inputs and outputs, not with underlying machinery.

## Layout

```sh
edw lock/unlock # locks or unlocks the datastore

edw network status # view the current network status
edw network list # lists all configured networks
edw network use <name|id> # switches to the subdatastore for said network
edw network rpc-config list # lists this network's RPC configs; the active one is marked
edw network rpc-config add <name> --url <url> [--use] # adds an RPC config. Active when none is, or when --use is set
edw network rpc-config use <name> # makes an RPC config the active one

edw profile list # lists all configured profiles
edw profile new # creates a new profile; on a new phrase, with an address vault for its first address, else like import
edw profile import # imports a profile from a mnemonic, with an address vault for each address up to the last used one (or the first, if none is used or the scan fails), and scans the chain's Tornado pools (stops after 20 consecutive unused nonces)
edw profile rescan [profile] # scans a profile's addresses again and adds an address vault for each one it is missing, and scans the chain's Tornado pools the same way
edw profile reveal-phrase # shows a profile's recovery phrase, at a terminal only

edw asset list [--profile SELECTOR] # the ERC-20s a profile tracks: the network's defaults, minus opt-outs, then its opt-ins
edw asset opt-in <address> [--profile SELECTOR] # tracks an ERC-20, or restores a default the profile opted out of
edw asset opt-out <address> [--profile SELECTOR] # stops tracking an ERC-20. A default is recorded as opted out; any other token is removed from its opt-ins. That hides its public balance and its Tornado notes
edw balance [profile] [--verbose] # each asset's public, private and total holdings; the private total includes Tornado notes. --verbose lists each vault and each unspent Tornado note
edw next-fresh-address [--peek] [--profile SELECTOR]
edw shield [--amount-formatted <eth> | --amount-raw <wei>] [--from INDEX] [--profile SELECTOR]
edw unshield [--amount-formatted <eth> | --amount-raw <wei>] [--to <address>] [--gas-payer INDEX] [--profile SELECTOR]
edw transfer --from <index|address|tornado> --to <address> --amount <eth> [--profile SELECTOR]
```

`shield`, `unshield`, and `transfer` move native ETH. At a terminal they ask for anything left off the command line, then wait for an explicit yes before broadcasting. `--non-interactive` asks for nothing: a missing required field is an error, and invoking the command is the authorization. Piped stdin supplies the missing fields and skips the confirmation.

`shield` and `unshield` take the amount as `--amount-formatted` (decimal ETH, such as `1.1`) or `--amount-raw` (integer wei). Neither flag is required: a terminal prompts for decimal ETH, and piped stdin supplies that same line. Profiles use all known native ETH pools by default, including Extended pools. An explicitly opted-out profile uses canonical pools only. Classic denominations are mainnet `0.1`, `1`, `10`, `100` ETH and Sepolia `0.1`, `1`, `10` ETH. Extended denominations on both chains are `0.01`, `0.03`, `0.3`, `3`, `30` ETH. Amounts must be exact sums of these denominations; `1.15` ETH is supported, while `1` wei is rejected. Shielding uses the fewest notes (for example, `33.34` ETH becomes `30 + 3 + 0.3 + 0.03 + 0.01`). Unshield also requires the profile to hold unspent notes that sum to that amount.

`unshield` without `--gas-payer` and `transfer --from tornado` submit one ERC-4337 user operation from the next fresh address. The largest selected note with paymaster support pays gas through the Tornado paymaster. That note's remainder, and every other note, land on the fresh address. Without `--to`, `unshield` leaves the remainder there. `unshield --to` and `transfer --from tornado` add a final call that forwards the remainder after the sponsor fee. A tornado transfer whose notes cannot cover `--amount` after that fee is an error. A selection with no paymaster-capable note is an error in this mode; use `unshield --gas-payer INDEX` for externally paid gas. Mainnet and Sepolia try `https://public.pimlico.io/v2/{chainId}/rpc`. `EDW_BUNDLER_URLS`, a comma-separated list, replaces that default. A failed estimate or send tries the next URL. The receipt is read from the bundler that accepted the operation. A `shield` of one note stays an ordinary transaction from the paying account. A `shield` of several notes is one user operation from that account, sent to the same bundler. The operation installs the EIP-7702 delegation when the account does not already have it, and the account pays gas from its own ETH. Any other batch of calls from an EOA uses that same user operation. A transfer from an index or address stays an ordinary transaction.

`next-fresh-address` prints the next standard address that this profile has not handed out and that has no nonce, code, or ETH, then stores it. `--peek` prints the same address and writes nothing. A declined command does not store a fresh address. `unshield` and `transfer --from tornado` store it once a bundler accepts the user operation, then wait for the receipt.

Pool sync tries the Saga CDN (`https://saga.gordosoluciones.xyz`, or `EDW_SAGA_URL`) and finishes any remaining blocks with `eth_getLogs`. Mainnet uses the base URL. Sepolia uses `{base}/sepolia`. Other chains use the RPC only. Import, rescan, and balance read every Tornado pool on the chain. Besides the ETH pools, mainnet includes DAI (100, 1,000, 10,000, 100,000), USDC (100, 1,000), USDT (100, 1,000), and WBTC (0.1, 1, 10); Sepolia includes DAI 100. USDC, USDT, DAI, and WBTC are defaults on mainnet, and DAI is a default on Sepolia. Every address vault is read for them unless the profile opts out.

### Global Args

```sh
--non-interactive, --porcelain # runs in non-interactive mode
--data-dir # runs with the specified data directory
```

### Extended Tornado pools

All pools are enabled by default, including for existing profiles without a saved preference. To create or import a canonical-only profile, explicitly opt out at creation:

```sh
edw profile new --name canonical --no-extended-pools
edw profile import --name canonical --phrase-file /path/to/phrase --no-extended-pools
```

The preference is saved per profile in the current network instance. Profiles on the same recovery phrase can choose independently. Profile listings label opted-out profiles `canonical pools only`; JSON profile summaries expose `use_extended_pools`. There is no per-operation override. The configured vault applies the preference consistently to recovery, balances, syncing, deposit packing and automatic note selection. SDK consumers configure `PoolSet` once; individual wallet commands ask the configured vault to plan deposits or select notes.

Canonical-only profiles exclude Extended pools from their active view without deleting stored notes or sync cursors. SDK-level preference changes take effect when reopening the vault; there is currently no CLI command to change an existing profile's preference. Canonical membership follows the SDK catalog: a future promotion makes that pool available to canonical-only profiles without changing its identity. This classification is not an anonymity score.

If Saga has no stream for a pool, the wallet reads its events from the configured RPC starting at its deployment block. Pending and spent notes are excluded from withdrawal selection; exact withdrawals minimize note count, and transfers minimize count then excess value.

The new pools have no configured privacy paymaster. A mixed withdrawal can use a selected classic note with paymaster support as its sponsor. To spend Extended-only holdings, pay gas from a funded standard account:

```sh
edw unshield --amount-formatted 0.03 --gas-payer 0 --to 0xYourRecipient --profile my-profile
```

Without `--to`, funds go to a fresh wallet address. The confirmation identifies the gas-paying account because its transaction publicly links it to the withdrawals. One note uses a regular transaction; multiple notes use the wallet's existing 7702 batching and bundler support. The gas payer must hold enough ETH for gas. This mode emits report `edw/unshield` version 3; paymaster withdrawals retain version 2. Tornado transfers still require a selected note with paymaster support; otherwise unshield with a gas payer first.

Profile report versions with the saved preference in their summaries are: list and rename version 2; new, import and rescan version 3. Balance is version 3.
