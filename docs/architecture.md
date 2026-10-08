# Architecture

`edw-core` owns every secret and all wallet logic. Frontends (`edw-cli` today, a daemon and a UI later) send requests and get results back, never key material.

## Layout

```sh
crates/core # edw-core: secrets, storage, chain access, profiles; no UI dependencies
crates/cli  # edw-cli: inputs and outputs only
crates/bin  # edw: the binary
contracts/  # Solidity (SimpleDelegate) and its Foundry tests
spec/       # this specification
```

## Core

```sh
Instance              # one unlocked network: its encrypted store, opened with the decryption password
Network               # a chain by network id; owns its native asset. One instance per network
NetworkEndpointConfig # a stored, named way to reach the network (http today; ipc, light clients later). One is active
NetworkEndpoint       # what a config resolves into at runtime; must serve the network id before use
Profile               # a user-named group of accounts on one recovery phrase
Signer                # signs for one address: seed-derived, hardware, remote
Executor              # sends transactions for one address: EOA, EIP-7702 delegate
Vault                 # holds assets behind deposit/withdraw: EOA, stealth address, shielded pool
Database              # byte key/value store: scoped -> encrypted -> file or memory backend
```

- `NetworkEndpoint` is a narrow trait the wallet owns, not alloy's `Provider`. Signatures may use alloy value types, never its transport machinery.
- Vault to vault transfers withdraw straight into the target when the source supports it, else withdraw to an ephemeral address and deposit, atomically.

## Storage

- Encryption is a decorator at the `Database` seam, so a backend never sees plaintext and there is one place to audit.
- A random data key roots every record. Password slots (Argon2id, 64 MiB, 3 passes, one lane) wrap it, so changing a password rewrites one header record.
- Each record gets its own HKDF-SHA256 encryption key and blinded storage key, sealed with XChaCha20-Poly1305 over the format version and logical key.
- Known gaps: record count and sizes are visible, deleting or rolling back a single record goes undetected, and blinded keys cannot be iterated by prefix.
- Secret types zero on drop and are never `Debug`, `Clone`, or `Serialize`.
