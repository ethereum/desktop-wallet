# Vocabulary

This document aims to outline used vocabulary, and its definitions.

## Mnemonic

A 12- or 24-word BIP39 English seed phrase stored per network instance, encrypted under that instance's decryption password. Mnemonics are un-named and numbered (`mnemonic 0`, `mnemonic 1`). Index 0 is created on first unlock together with profile 0.

Users name **profiles**, not mnemonics. `edw profile generate` / `import` create a new seed and exactly one profile at a given index (default 0). They do not also create profile 0 when another index is requested. `edw profile add` creates another profile on an existing mnemonic (`--index`, default 0, or `--next` for the smallest unused index). Importing a phrase that is already stored is an error; add a profile on that mnemonic instead. On import, the CLI derives the profile's standard EOAs (`m/44'/60'/<profileIndex>'/0/<addressIndex>`), inspects them on the unlocked network in batches of 10, and continues until a fully unused batch. An EOA counts as used when its nonce is non-zero, it has code, or it has a native ETH balance. It prints addresses through the last used index and `next unused eoa index` (not stored yet).

A **profile** points at a mnemonic by `(mnemonic_index, profile_index)` and does not store the phrase. `profile_index` is the hardened BIP44 account in `m/44'/60'/<profileIndex>'/…`.

## Profile

A user-facing collection of **signers**, **executors**, and **vaults**. Used to manage and group balances, transactions, history, and other related data. Display names are unique in a network instance. Unnamed index 0 displays as `default`; unnamed later indexes display as `profile #<index>`.

Example:

- A user has a profile that contains a **signer** and **executor** for their EOA address, a **vault** for their hardware wallet, and a **vault** for their meta stealth address.

## Signer

An abstract object associated with a public key that can sign messages.

Example:

- A user has a **signer** for a private key stored on-device.
- A user has a **signer** on a remote signing service (e.g. turnkey).

## Executor

An abstract object associated with an on-chain address that can send transactions for that address.

Example:

- A user has an **executor** for their EOA address.
- A user has an **executor** for a 4337 smart account.

## Vault

An abstract object that has some balance of assets, can be deposited into (increasing the balance), and withdrawn from (decreasing the balance).

Example:

- A user has a **vault** for their EOA address.
- A user has a **vault** for their multisig smart account.
- A user has a **vault** for their tornadocash shielded balance.

## Asset

Assets are configured wallet-wide and opted-in to on a per "account" basis.
Prefer 'asset' over 'token' or 'currency'.

### Metadata

Asset information such as **decimals**, **symbol**, and **name** are fetched when the asset is first introduced.

### Balance

The amount of an **asset** held by some object (e.g. a **vault** or **profile**).

### Value

The **value** of a **balance** is the amount said balance quotes out to be in the **display currency**.

### Display Currency

The users preferred **asset** to view their estimates in.
This should be properly formatted to the users **locale**

## Network

A network, sometimes referred to as "chain" aims to track a specific network id.
Each supported network is a separate wallet instance: its own directory, decryption password, and profiles. Unlocking selects one instance; there is no wallet-wide network list.
A **networkConfig** is a named row in that instance (RPC endpoints and other settings). Every networkConfig is forced to the instance chain ID; you can keep several with different names and RPCs.

### Endpoint

A **network endpoint** is a given RPC or mechanism for connecting to the network.
Each endpoint instance is a single RPC either http, ws, or ipc.
For each Network one Network Endpoint is active at a time to provide a stable source of data.

## Flashcall

The atomic execution of a set of calls that (1) fund an address, (2) interact with a contract, and (3) defund the address, all in a single transaction.

Examples:

- Flashcall uniswap by withdrawing USDC from tint, swapping USDC to ETH, and depositing ETH into Tornadocash. 