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

## The core public API contract

The surface the UI is allowed to call. Suggested workflow: define or extend this surface
first for any feature, and review changes to it at the highest bar (`interface` label).
Signatures are the _shape_ of the contract, not final code.

### Session & lifecycle

```rust
/// The unlocked wallet. Holds the seed in a Zeroizing buffer. Never Clone/Debug/Serialize.
pub struct Wallet { /* private: seed material */ }

impl Wallet {
    pub fn generate(words: WordCount) -> Result<Wallet>;
    pub fn from_phrase(phrase: &str) -> Result<Wallet>;
    pub fn validate_phrase(phrase: &str) -> PhraseStatus; // live checksum/word-count feedback
    pub fn expose_phrase(&self) -> Zeroizing<String>; // gated reveal only; caller must not persist
    // NOTE: no getter returns raw private keys across the boundary. By design.
}
```

### Profiles & derivation (the User Namespace Convention)

A **Profile** is the user-facing unit ("identity" / "wallet"); most users have one. Each
Profile is one BIP-44 `account'` leaf (`x'`) and anchors a whole credential bundle. The UI
talks in Profiles; addresses are an invisible default underneath.

```rust
/// One Profile = one BIP-44 account' leaf (x'). The namespace anchor.
pub struct AccountBundle { /* ... */ }

impl Wallet {
    pub fn profile(&self, x: u32) -> Result<AccountBundle>; // derive bundle for account' = x
    pub fn address(&self, profile: u32, index: u32) -> Result<Address>; // m/44'/60'/x'/0/i
}

impl AccountBundle {
    pub fn identity_anchor(&self) -> Address; // x'/0/0 - DELIBERATELY public
    pub fn public_address(&self, i: u32) -> Address; // x'/0/i - meant to stay delinked
    pub fn stealth_meta_address(&self) -> StealthMetaAddress; // ERC-5564 "st:eth:0x…"
    pub fn smart_account(&self) -> SmartAccountInfo; // signer + CREATE2 salt (+ deployed addr)
    pub fn shielded(&self, pool: PoolId) -> Result<ShieldedKeys>; // registry-driven (BabyJubJub, …)
}
```

> **Conformance obligation lives here.** As the reference implementation, the derivation tree
> is meant to become a standard: once its details are settled, any conforming wallet should
> reproduce it bit-for-bit from the same seed. The plan is to cover every path with a
> committed **known-answer test vector** (M0) and to write the full derivation/namespace
> convention into a public convention doc (M1), so the spec stays self-contained.
>
> **Open sub-decision to resolve in M0:** hardened vs non-hardened stealth leaf. The two
> descriptions we inherited disagree; the reference implementation must pick one, document
> it, and lock it with a test vector.

### Signing

```rust
impl Wallet {
    pub fn sign_message(&self, profile: u32, index: u32, msg: &[u8]) -> Result<Signature>;
    pub fn sign_eth_transfer(&self, profile: u32, index: u32, to: Address, value: U256,
                             nonce: u64, fees: FeeSettings, chain_id: u64,
                             gas_limit: u64) -> Result<Bytes>; // returns signed raw bytes
    pub fn sign_tx(&self, profile: u32, index: u32, req: TxRequest) -> Result<Bytes>; // contract calls (M0)
    pub fn sign_user_op(&self, profile: u32, req: UserOpRequest) -> Result<SignedUserOp>; // 4337 (M3)
}
```

A key technique to consider: sign with alloy 2.x and hand the light client only raw
bytes (`ChainClient::send_raw`), so the alloy 1.x / 2.x type boundary between our code and
Helios is never crossed. This property should hold for every new signed-object type.

### Chain access: trust-minimized and private reads

```rust
pub trait ChainClient {
    fn balance(&self, addr: Address) -> Result<U256>;
    fn nonce(&self, addr: Address) -> Result<u64>;
    fn suggested_fees(&self) -> Result<FeeSettings>; // TODO M0: from verified base fee
    fn chain_id(&self) -> Result<u64>;
    fn send_raw(&self, raw: &[u8]) -> Result<B256>;
    fn confirm_inclusion(&self, tx: B256) -> Result<InclusionStatus>; // TODO M0
    fn history(&self, addr: Address) -> Result<Vec<TxSummary>>; // TODO M0
    fn logs(&self, filter: LogFilter) -> Result<Vec<Log>>; // stealth scanning (M2)
}
```

The proposed implementation, `LightClient` (in-process Helios), is trust-minimized; the
_only_ network egress is RPC (principle 2). **Private reads (principle 3)** are a property of _how_ this
trait is used, and a design task in its own right (M1): the RPC provider must not be able to
trivially correlate a user's addresses. Candidate mechanisms (decide via spike): per-address
request isolation, batching/decoy queries, endpoint rotation, or routing reads for different
Profiles/addresses over separate connections. The trait boundary is what lets us swap in a
more private read layer without touching the UI. A known gotcha (if Helios is used):
depend on `helios-ethereum` rather than the umbrella `helios` crate, which pulls a yanked
transitive dep. `alloy-primitives` unifies across the boundary; `alloy-eips` does not.

### Database

The Database is how the program manages persistent state. The database is broken into two parts layers:
1. The repository trait impls, which handles data serialization & encryption and provides a high-level public interface.
2. The base sql pool, which is an internal connection to the underlying database and handles sql queries.

```rust
/// Example methods, not exhaustive or necessarily correct.
trait Repository {
    async fn get_profile(&self, profile_id: u32) -> Result<Profile>;
    async fn save_profile(&self, profile: &Profile) -> Result<()>;
    async fn get_transactions(&self, profile_id: u32) -> Result<Vec<Transaction>>;
    async fn get_wallet_transactions(&self, wallet_id: u32) -> Result<Vec<Transaction>>;
    async fn save_transaction(&self, transaction: &Transaction) -> Result<()>;
    // ...
}

/// Consider using https://docs.rs/sqlx/latest/sqlx/trait.Database.html.
trait Connection {
    async fn query(&self, ...) -> Result<Row>;
    async fn execute(&self, ...) -> Result<()>;
    // ...
}
```

#### Encryption & Security

The connection trait should not be assumed to encrypt or secure any data. The repository trait impl should handle encryption and decryption of sensitive data before saving to the database.

For some targets, different underlying storage connections may be required for sensitive data. For example mobile platforms may use the OS keychain or secure enclaves. This is considered out-of-scope for the initial implementation.

### Privacy state & fund classification

```rust
pub enum FundClass { IdentityLinked, Delinked, Mixed } // principle 7: first-class

pub struct PrivacyState { // principle 6: honest signaling
    pub class: FundClass,
    pub linked_to_identity: bool,
    pub anonymity_set: Option<AnonymitySetEstimate>, // current pool set size
}

pub trait FundClassifier {
    fn classify(&self, addr: Address) -> FundClass;
    fn privacy_state(&self, addr: Address) -> PrivacyState;
    /// Correlation guard: would this action link a private branch to the identity anchor?
    fn correlation_risk(&self, from: Address, to: Address) -> CorrelationRisk;
}
```

The intent (principle 8): the UI consults `correlation_risk` before any send and surfaces
the result at signing time. This is where the footgun guards live.

### Mixer: deposit, withdraw, and auto-mix-back

```rust
pub trait Mixer {
    fn deposit(&self, from: Address, pool: PoolId) -> Result<DepositTicket>;
    fn withdraw(&self, note: ShieldedNote, to: Address, gas: GasStrategy) -> Result<B256>;
    fn shielded_balance(&self, profile: u32) -> Result<U256>;
    fn anonymity_set(&self, pool: PoolId) -> Result<AnonymitySetEstimate>;
    /// Background/auto mix-back: unmixed funds return to the pool wherever possible.
    fn plan_mix_back(&self, profile: u32) -> Result<Vec<MixBackAction>>;
}

pub enum GasStrategy { // see the private-gas design (M3)
    SelfRelayGasTank, // Option 1 - today-deployable, no new infra
    PermissionlessFeeMarket, // Option 2 - trust-minimized generalization
}
```

`plan_mix_back` powers principle 7's nudge and any background-mixing behavior. Timing/dust
fingerprints are a real leak; the plan must vary timing and let the gas tank age (a
documented footgun).

### Dapp session: private by default

```rust
/// Minimal, "good enough" dapp connection. Default: FRESH address + MIXED funds.
pub trait DappSession {
    /// Connect to a dapp using a fresh, delinked address for this Profile.
    fn connect(&self, profile: u32, origin: DappOrigin) -> Result<SessionHandle>;
    /// Approve a dapp-requested transaction, funded from mixed funds where possible.
    fn approve(&self, session: SessionHandle, req: TxRequest) -> Result<Bytes>;
    fn disconnect(&self, session: SessionHandle);
}
```

Scope is deliberately minimal (vision doc): the smallest flow that demonstrates "private by
default _and_ generalistic," not a dapp browser. Which transport (injected provider,
WalletConnect-style (openlv), or a local bridge) is an M4 spike; whatever it is, it must respect the
only-RPC-egress principle.

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
