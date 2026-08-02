# 01 - Architecture & Contracts

> This doc proposes the system decomposition and the **contracts between components**. The
> contracts are the point: agree them first, and UI + core work proceeds in parallel. All of
> this serves the principles in [`00-vision.md`](./00-vision.md), especially **"secret
> material never leaves the core."**
>
> As the ecosystem's **reference implementation**, these contracts are also a _spec other
> wallets read_. Keep them clean and general, not tuned to internal convenience.
>
> **Status: DRAFT** (circulating for team review). The decomposition and API below are a
> **proposal**, offered as something concrete to react
> to. The one firm commitment is the **invariant** (secret material never leaves the core);
> everything else (the crate split, the API signatures, the chosen libraries, the milestone
> tags) is a starting point for discussion, not a settled decision. Items flagged
> _(open for review)_ are the least settled of all.

## The one invariant

```text
┌────────────────────────────────────────────────────────────────────┐
│                          UI / view layer                           │
│                        (stack under review)                        │
│            profiles, balances, actions, boundary flags             │
│                    holds NO raw secret material                    │
└──────────────────────────────────┬─────────────────────────────────┘
                                   ↑
                                   │  request  (UI -> core): derive / sign / classify / mix
                                   │  response (core -> UI): result only, never raw keys
                                   ↓
┌──────────────────────────────────┴─────────────────────────────────┐
│   wallet-core  (trusted, ZERO ui deps): secrets live only here     │
│                                                                    │
│                   ┌────────────────────────────┐                   │
│                   │     narrow public API      │                   │
│                   └────────────────────────────┘                   │
│                                                                    │
│    - Vault                       - FundClassifier / PrivacyState   │
│    - ChainClient                 - Mixer                           │
│    - Derivation engine           - DappSession                     │
│    - Profile / AccountBundle                                       │
│    - Signer ops                                                    │
└──────────────────────────────────┬─────────────────────────────────┘
                                   │
                                   │  Vault       -> secret storage
                                   │  ChainClient -> chain access
                                   │  Derivation  -> registries
                                   ↓
┌──────────────────────────────────┴─────────────────────────────────┐
│   external to wallet-core                                          │
│                                                                    │
│   - secret storage: keychain, hardware signer                      │
│   - chain access: Helios light client + private reads              │
│   - registries as DATA: derivation / address schemes               │
└────────────────────────────────────────────────────────────────────┘
```

The presentation layer requests operations and receives results; it never holds raw secret
material. This boundary is **Rust-to-Rust** (not the process/language boundary Tauri would
give for free), so we **enforce it by module structure and discipline**: `wallet-core`
exposes a deliberately narrow public API and keeps all secret-touching types private. Treat
the view layer as untrusted from the key material's perspective.

## Repository / crate layout

This repo (official):

```
desktop-wallet/
├── crates/        # Rust crates - the secure core and its focused sub-crates (see split below)
├── ui/            # the view layer (stack under review - see Stack)
├── nix/ flake.nix # reproducible dev shell (Rust toolchain + tooling)
├── spec/          # this specification
└── .github/       # CODEOWNERS, CI
```

A **proposed** shape for the security-critical core in `crates/`: a `wallet-core` crate
covering a seed holder (`wallet`), per-Profile derivation (`account`), the `ChainClient` /
Helios / Kohaku seam (`provider`), an in-process light client (`helios_client`), and an
at-rest vault (`vault`, e.g. Argon2id + XChaCha20-Poly1305), behind a shared `error` type.
However the internals land, `wallet-core` should keep **zero UI dependencies**: the property
to preserve regardless of the UI-stack decision.

A possible later step (proposed): split `wallet-core` into focused crates, so audit
boundaries stay crisp and compile times stay low:

```
crates/
├── wallet-core/     # facade: re-exports the stable public API the UI depends on
├── wallet-keys/     # seed, derivation engine, signers, zeroize discipline  (highest audit bar)
├── wallet-vault/    # at-rest encryption + OS-keychain convenience layer
├── wallet-chain/    # ChainClient trait, Helios light client, PRIVATE-read layer
├── wallet-registry/ # derivation/address registries as DATA (mixer, smart-account)
└── wallet-privacy/  # stealth (ERC-5564), shielded pools (Kohaku), gas + mix-back, dapp session
```

The **facade crate** matters: the UI imports only `wallet-core`, so internal restructuring
never breaks the view layer as long as the facade's public API is stable. This is the
contract; as a reference implementation, it is also the API other wallets study.

## Core API

The following section describes the core API for the program. The API is designed to be minimal and generic across different implementations.

Different trait impls are assumed to have different constructors, which are not part of the trait. Once initialized, the trait impls are expected to be used generically and should never be disambiguated by their concrete type.

### Ethereum Provider

Ethereum JSON-RPC interface. Used by the wallet to query chain state and submit transactions, and by dapps to interact with the wallet.

Based on alloy's [`Provider`](https://docs.rs/alloy/latest/alloy/providers/trait.Provider.html) trait. Consider directly using alloy, but it may be better to define a minimal interface to avoid a hard dependency on alloy which can be quite large. Trait implementations may include:
    - Remote JSON-RPC providers (e.g. Infura, Alchemy)
    - Self-hosted nodes (e.g. Reth, Geth, Erigon)
    - Light clients (e.g. Helios)
    - Local VMs (e.g. Anvil, revm)

```rust
trait EthereumProvider {
    // ...
}
```

#### Dapp Sessions

Dapp sessions are how dapps interact with the wallet. When connecting, the wallet and dapp establish a secure transport over which the wallet exposes an EthereumProvider impl. Dapps can then query this EthereumProvider for network data or to submit requests. Trait implementations may include:
    - OpenLV
    - WalletConnect

### Profile

Profiles are how the program manages user-facing identity and state. A profile is a collection of collectively-managed wallets, signers, and vaults. Users may use profiles to logically organize their assets and identities (e.g. manage assets, sign messages, interact with dapps, and send transactions).

Profiles are primarily a UI-level abstraction, used to collectively expose many lower-level objects. They defer to:
    - Signers for signing messages
    - Wallets for sending transactions
    - Vaults for storing assets

### Signer

Signers are how the program signs messages. A signer is associated with and can sign messages for a specific address. Trait implementations may include:
    - Local signers (e.g. derived from a seed phrase or private key)
    - Hardware signers (e.g. Ledger, Trezor)
    - Remote signers (e.g. OpenLV, WalletConnect)

```rust
trait Signer {
    fn id(&self) -> SignerId;
    fn public_key(&self) -> PublicKey;
    async fn sign_message(&self, msg: &[u8]) -> Result<Signature>;
    async fn personal_sign(&self, msg: &[u8]) -> Result<Signature>;
    async fn sign_typed_data(&self, domain: &EIP712Domain, types: &EIP712Types, value: &EIP712Value) -> Result<Signature>;
}
```

### Wallet

Wallets are how the program sends transactions. A wallet is associated with and can send transactions for a specific address. The wallet trait is based on [EIP-5792](https://eips.ethereum.org/EIPS/eip-5792). Trait implementations may include:
    - EOAs (e.g. derived from a seed phrase or private key)
    - Hardware wallets (e.g. Ledger, Trezor)
    - Remote accounts (e.g. OpenLV, WalletConnect)
    - Smart accounts (e.g. ERC-4337, ERC-7702)

```rust
trait Wallet {
    fn id(&self) -> WalletId;
    fn address(&self) -> Address;
    fn public_key(&self) -> PublicKey;
    async fn send_calls(&self, calls: &[Call]) -> Result<CallsId>;
    async fn get_calls_status(&self, calls_id: CallsId) -> Result<CallsStatus>;
}
```

### Vault

Vaults are how the program stores assets. A vault is an abstract collection of assets that can be deposited into and withdrawn from. Trait implementations may include:
    - Local vaults (e.g. derived from a seed phrase or private key)
    - Hardware vaults (e.g. Ledger, Trezor)
    - Remote vaults (e.g. OpenLV, WalletConnect)
    - [Stealth Addresses](https://eips.ethereum.org/EIPS/eip-5564)
    - Privacy Protocols (e.g. Tornado Cash, Railgun)

```rust
trait Vault {
    fn id(&self) -> VaultId;
    async fn balance(&self) -> Result<HashMap<AssetId, U256>>;
    async fn balance_of(&self, asset_id: AssetId) -> Result<U256>;

    /// Returns a Call that, when executed from any address, will withdraw the specified `amount` of 
    /// the given `asset_id` from the vault to the `to` address.
    async fn withdraw(&self, to: AccountId, asset_id: AssetId, amount: U256) -> Result<Call>;

    /// Returns a Call that, when executed from the `from` address, will deposit 
    /// the specified `amount` of the given `asset_id` into the vault.
    async fn deposit(&self, from: Address, asset_id: AssetId, amount: U256) -> Result<Call>;

    /// Returns a list of asset constraints that the vault supports.
    fn supported_assets(&self) -> Result<AssetConstraint>;

    // Returns an ordered list of operations.
    async fn history(&self) -> Result<Vec<VaultOperation>>;
}

/// An asset constraint defines a set of assets that a vault supports.
///
/// Examples:
/// - A vault that supports any ERC-20 token with any amount.
/// - A vault that only supports a subset of ERC-20 tokens with any amount.
/// - A vault that only supports a specific ERC-20 token with a specific amount.
struct AssetConstraint {
    pub constraints: Vec<AssetTypeConstraint>,
}

struct AssetTypeConstraint {
    pub asset_type: AssetType,
    pub token_id: TokenIdConstraint,
    pub amount: AmountConstraint,
}

enum TokenIdConstraint {
    /// Supports any token ID.
    Any,
    /// Supports any token ID in the specified set.
    Include(BTreeSet<Bytes>),
    /// Supports any token ID excluding those in the specified set.
    Exclude(BTreeSet<Bytes>),
}

enum AmountConstraint {
    Any,
    Range { min: U256, max: U256 },
    Discrete(BTreeSet<U256>),
}
```

Vaults are a significant abstraction over regular asset management. Rather than having the program reason about how different storage media manage assets (e.g. an EOA may call `transfer` / `transferFrom`, while a privacy protocol may `deposit` / `withdraw`), the vault trait provides a simple unified interface.

**Benefits**
- Singular interface for asset management across different storage media.
- Supports several exotic asset management protocols as first-class citizens.
- Offloads asset-specific logic to the vault implementations, including:
  - Balance tracking
  - Deposit and withdrawal logic
  - Compatibility constraints
- Improved security by storing at-rest assets in a vault, which will interact with fewer external contracts and may have specialized properties (e.g. improved privacy, limits, timelocks, stricter signing requirements).

**Drawbacks**
- Implementation details leak through the abstraction. Namely:
  - Some vault impls may support more efficient asset transfers for supported recipients.
  - Some vault impls may only support a subset of asset types, a subset of assets within a type, or even a subset of asset amounts. For example, a Tornado Cash vault will only support depositing and withdrawing a single asset type in a single denomination.

#### Asset Management

Vaults are the encouraged way to manage assets in the program. The program will still support asset management through Wallet impls (e.g. for Dapp Sessions), but the program will encourage users to return assets to vaults for long-term storage.

#### Inter-Vault Transfers

The program must be able to easily transfer assets between arbitrary vault implementations. This is challenging because we don't want to force each vault to know about every other impl. To solve this, we take a two-step approach. When transferring between vaults, the program will:
1. Attempt to `withdraw` from the source vault into the destination vault. If the source vault supports this recipient, it will return a `Call` that can be executed to perform the transfer.
2. If the source vault does not support the destination vault, the program will `withdraw` from the source vault into a temporary ephemeral account, and then `deposit` into the destination address. The two calls can be executed atomically to perform the transfer.

This way each vault only needs to know how to transfer to / from an ethereum address, but can still support specialized transfer logic for specific recipients. For example:
- `Address`<->`Address` transfers can be done with a single `withdraw` call, since all vaults support Addresses.
- `Tornado Cash`<->`PPV2` transfers can be done with two calls, one to withdraw from Tornado Cash and a second to deposit into PPV2.
- `PPV2`<->`PPV2` transfers can be done with a single `withdraw` call, since the PPV2 vault supports transferring to itself.

### At-rest storage

```rust
impl Database {
    pub fn save(path, phrase: &Zeroizing<String>, password: &str) -> Result<()>;
    pub fn load(path, password: &str) -> Result<Zeroizing<String>>; // wrong pw fails via AEAD tag
    pub fn exists(path) -> bool;
    pub fn delete(path) -> Result<()>;
}

/// Convenience layer, M0. Stores the DERIVED KEY (never the mnemonic). Absence degrades
/// gracefully to password prompt - never a hard dependency.
pub trait SecretStore {
    fn seal(&self, key: &DerivedKey) -> Result<()>;
    fn unseal(&self) -> Result<Option<DerivedKey>>;
    fn clear(&self) -> Result<()>;
}
```

## Registries as data

Derivation and address-computation schemes (mixer pools, smart-account implementations) are
**registry entries the core reads**, not hardcoded branches. Supporting a new shielded pool
or smart-account type should be a registry entry + test vectors, not a core rewrite. Build
the pluggable table in `wallet-registry`; keep registries versioned and committed so
conformance is verifiable.

## Stack (proposed, open for review)

The **core** stack below is a proposal that looks low-risk to keep; the
**UI** stack is the genuinely open decision (see vision decision 5).

- **UI _(open for review)_:** one option is Dioxus 0.7 (`rsx!`), a desktop target (system
  WebView via Wry), which favors a no-JS, fully-auditable supply chain, at the cost of
  engineering the UI/secret boundary ourselves (the module discipline above) rather than
  getting it from a process split. **However**, this repo's dev shell
  provisions a **Node/pnpm/Chromium + Playwright** toolchain, which points at either a
  web-based UI (e.g. a Tauri-style webview) or browser-driven E2E testing. The team should
  **decide the UI stack explicitly** and, with it, how strict the supply-chain principle is.
  Whatever wins, the invariant holds: the view layer imports only `wallet-core` and never
  touches secret material.
- **Ethereum:** `alloy` 2.x. **Chain reads:** `helios-ethereum` light client, in-process.
- **At rest:** Argon2id (64 MiB / 3-pass) + XChaCha20-Poly1305, versioned vault blob.
- **Privacy stack:** Kohaku crates (Rust, git-only 0.1.0, unstable; going native bets on
  them) for shielded pools; ERC-5564 for stealth.

## Cross-cutting engineering standards

Proposed standards to adopt or adjust as the code lands:

- **Zeroize discipline:** every type holding secret material is `Zeroizing`/zeroize-on-drop,
  never `Debug`/`Clone`/`Serialize`-able in a way that copies the secret. Reviewed on every
  `wallet-keys`/`wallet-vault` PR.
- **Only-RPC egress + no secrets in logs + no telemetry** (principles 2, 11). Enforce with a
  review check and, where feasible, a test/lint that fails on unexpected network hosts.
- **Testing:** `wallet-core` logic should carry unit tests; derivation should have committed
  known-answer vectors; user-visible flows should be driven live in the running app before
  "done."
- **CI (M0 deliverable):** build + clippy + test on macOS/Linux/Windows; deny warnings in
  `wallet-core`; dependency audit (`cargo audit`/`cargo deny`) given the supply-chain
  principle.
- **Security review gate:** any PR touching keys, signing, storage, derivation, mixing, or
  the trust boundary requires a second reviewer signing off specifically on secret handling.
