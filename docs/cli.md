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
edw profile new # creates a new profile
edw profile import # imports a profile from a mnemonic
```

### Global Args

```sh
--non-interactive, --porcelain # runs in non-interactive mode
--data-dir # runs with the specified data directory
--rpc-url <url> # overrides the active endpoint for one command (weird and should be replaced, we arent this stateless)
```
