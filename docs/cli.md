# CLI Interface

The `edw-cli` crate implements the `edw` CLI interface.
The crate is concerned purely with inputs and outputs, not with underlying machinery.

## Layout

```sh
edw lock/unlock # locks or unlocks the datastore

edw network status # view the current network status
edw network list # lists all configured networks
edw network use <name|id> # switches to the subdatastore for said network
edw network endpoint list # lists all configured network endpoints
edw network endpoint use <name|id> # switches to the specified network endpoint

edw profile list # lists all configured profiles
edw profile new # creates a new profile; on a new phrase, with an address vault for its first address, else like import
edw profile import # imports a profile from a mnemonic, with an address vault for each address up to the last used one (or the first, if none is used or the scan fails)
edw profile rescan [profile] # scans a profile's addresses again and adds an address vault for each one it is missing
edw profile reveal-phrase # shows a profile's recovery phrase, at a terminal only

edw balance [profile] [--verbose] # each asset's public, private and total holdings across a profile's vaults; --verbose adds each vault
edw next-fresh-address [--peek] [--profile SELECTOR]
edw shield [--amount-formatted <eth> | --amount-raw <wei>] [--from INDEX] [--profile SELECTOR]
edw unshield [--amount-formatted <eth> | --amount-raw <wei>] [--to <address>] [--profile SELECTOR]
edw transfer --from <index|address|tornado> --to <address> --amount <eth> [--profile SELECTOR]
```

`shield`, `unshield`, and `transfer` move native ETH. At a terminal they ask for anything left off the command line, then wait for an explicit yes before broadcasting. `--non-interactive` asks for nothing: a missing required field is an error, and invoking the command is the authorization. Piped stdin supplies the missing fields and skips the confirmation.

`shield` and `unshield` take the amount as `--amount-formatted` (decimal ETH, such as `1.1`) or `--amount-raw` (integer wei). Neither flag is required: a terminal prompts for decimal ETH, and piped stdin supplies that same line. The amount has to be an exact sum of the chain's Tornado denominations (mainnet `0.1`, `1`, `10`, `100` ETH; Sepolia `0.1`, `1`, `10` ETH). `1.15` ETH and `1` wei are rejected. Unshield also requires the profile to hold unspent notes that sum to that amount.

`unshield` and `transfer --from tornado` submit one ERC-4337 user operation from the next fresh address. The largest note pays gas through the Tornado paymaster. That note's remainder, and every other note, land on the fresh address. Without `--to`, `unshield` leaves the remainder there. `unshield --to` and `transfer --from tornado` add a final call that forwards the remainder after the sponsor fee. A tornado transfer whose notes cannot cover `--amount` after that fee is an error. A pool with no paymaster is an error. Mainnet and Sepolia try `https://public.pimlico.io/v2/{chainId}/rpc`. `EDW_BUNDLER_URLS`, a comma-separated list, replaces that default. A failed estimate or send tries the next URL. The receipt is read from the bundler that accepted the operation. `shield` and a transfer from an index or address stay ordinary transactions.

`next-fresh-address` prints the next standard address that this profile has not handed out and that has no nonce, code, or ETH, then stores it. `--peek` prints the same address and writes nothing. A declined command does not store a fresh address. `unshield` and `transfer --from tornado` store it once a bundler accepts the user operation, then wait for the receipt.

Pool sync tries the Saga CDN (`https://saga.gordosoluciones.xyz`, or `EDW_SAGA_URL`) and finishes any remaining blocks with `eth_getLogs`. Mainnet uses the base URL. Sepolia uses `{base}/sepolia`. Other chains use the RPC only.

### Global Args

```sh
--non-interactive, --porcelain # runs in non-interactive mode
--data-dir # runs with the specified data directory
--rpc-url <url> # overrides the active endpoint for one command (weird and should be replaced, we arent this stateless)
```
