# Vocabulary

This document aims to outline used vocabulary, and its definitions.

## Profile

A user-facing collection of **signers**, **executors**, and **vaults**. Used to manage and group balances, transactions, history, and other related data.

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
Networks are configured wallet-wide.

### Endpoint

A **network endpoint** is a given RPC or mechanism for connecting to the network.
Each endpoint instance is a single RPC either http, ws, or ipc.
For each Network one Network Endpoint is active at a time to provide a stable source of data.

## Flashcall

The atomic execution of a set of calls that (1) fund an address, (2) interact with a contract, and (3) defund the address, all in a single transaction.

Examples:

- Flashcall uniswap by withdrawing USDC from tint, swapping USDC to ETH, and depositing ETH into Tornadocash. 