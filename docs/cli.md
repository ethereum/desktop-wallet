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
edw network rpc-config add <name> --url <url> # adds an RPC config, active when none is
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
edw unshield [--amount-formatted <eth> | --amount-raw <wei>] [--to <address>] [--profile SELECTOR]
edw transfer --from <index|address|tornado> --to <address> --amount <eth> [--profile SELECTOR]
```

`shield`, `unshield`, and `transfer` move native ETH. At a terminal they ask for anything left off the command line, then wait for an explicit yes before broadcasting. `--non-interactive` asks for nothing: a missing required field is an error, and invoking the command is the authorization. Piped stdin supplies the missing fields and skips the confirmation.

`shield` and `unshield` take the amount as `--amount-formatted` (decimal ETH, such as `1.1`) or `--amount-raw` (integer wei). Neither flag is required: a terminal prompts for decimal ETH, and piped stdin supplies that same line. The amount has to be an exact sum of the chain's Tornado denominations (mainnet `0.1`, `1`, `10`, `100` ETH; Sepolia `0.1`, `1`, `10` ETH). `1.15` ETH and `1` wei are rejected. Unshield also requires the profile to hold unspent notes that sum to that amount.

`unshield` and `transfer --from tornado` submit one ERC-4337 user operation from the next fresh address. The largest note pays gas through the Tornado paymaster. That note's remainder, and every other note, land on the fresh address. Without `--to`, `unshield` leaves the remainder there. `unshield --to` and `transfer --from tornado` add a final call that forwards the remainder after the sponsor fee. A tornado transfer whose notes cannot cover `--amount` after that fee is an error. A pool with no paymaster is an error. Mainnet and Sepolia try `https://public.pimlico.io/v2/{chainId}/rpc`. `EDW_BUNDLER_URLS`, a comma-separated list, replaces that default. A failed estimate or send tries the next URL. The receipt is read from the bundler that accepted the operation. `shield` and a transfer from an index or address stay ordinary transactions.

`next-fresh-address` prints the next standard address that this profile has not handed out and that has no nonce, code, or ETH, then stores it. `--peek` prints the same address and writes nothing. A declined command does not store a fresh address. `unshield` and `transfer --from tornado` store it once a bundler accepts the user operation, then wait for the receipt.

Pool sync tries the Saga CDN (`https://saga.gordosoluciones.xyz`, or `EDW_SAGA_URL`) and finishes any remaining blocks with `eth_getLogs`. Mainnet uses the base URL. Sepolia uses `{base}/sepolia`. Other chains use the RPC only. Import, rescan, and balance read every Tornado pool on the chain. Besides the ETH pools, mainnet includes DAI (100, 1,000, 10,000, 100,000), USDC (100, 1,000), USDT (100, 1,000), and WBTC (0.1, 1, 10); Sepolia includes DAI 100. USDC, USDT, DAI, and WBTC are defaults on mainnet, and DAI is a default on Sepolia. Every address vault is read for them unless the profile opts out.

### Global Args

```sh
--non-interactive, --porcelain # runs in non-interactive mode
--data-dir # runs with the specified data directory
```
