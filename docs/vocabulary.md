# Vocabulary

This document aims to outline used vocabulary, and its definitions.

## Mnemonic

A BIP39 seed phrase.

## Profile

A user-facing collection of **signers**, **executors**, and **vaults**.
Used to manage and group balances, transactions, history, and other related data.
A profile points at a **mnemonic** and a profile index, the hardened BIP-44 account in `m/44'/60'/<profileIndex>'/…`, and never stores the phrase.

Example:

- An EOA: a **signer** for its key, an **executor** to send from it, and a **vault** for its balance.
- A hardware wallet: the same three, with the **signer** on the device.
- A stealth address the user received funds at: the same three, with the **signer** holding the key derived for that address.
- A Tornado Cash balance: a **vault** that derives its notes with a **signer**, and whose withdrawals one of the profile's **executors** sends.

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

Assets are configured wallet-wide. A profile tracks the network's defaults, can opt out of them, and can opt into more.
Prefer 'asset' over 'token' or 'currency'.

### Metadata

Asset information such as **decimals** and **symbol** is fetched when the asset is first introduced.

### Balance

The amount of an **asset** held by some object (e.g. a **vault** or **profile**).

### Value

The **value** of a **balance** is the amount said balance quotes out to be in the **display currency**.

### Display Currency

The users preferred **asset** to view their estimates in.
This should be properly formatted to the users **locale**

## Network

A **network** is a chain identified by its network id.

### RPC config

An **RPC config** is a stored, named way to reach the network. It can hold one URL today and more than one URL, or another kind of connection, later. A network has any number of them and one is active. Mainnet starts on `https://ethereum.publicnode.com` and Sepolia on `https://ethereum-sepolia-rpc.publicnode.com`, both named `publicnode`.

A config resolves at runtime into a connection that must serve that network.

## Flashcall

The atomic execution of a set of calls that (1) fund an address, (2) interact with a contract, and (3) defund the address, all in a single transaction.

Examples:

- Flashcall uniswap by withdrawing USDC from tint, swapping USDC to ETH, and depositing ETH into Tornadocash.
