# 02 - Milestone Backlog

> This is a **proposal**, seeded from three places:
> [issue #15 (`v0.1.0 proposal`)](https://github.com/ethereum/desktop-wallet/issues/15) for
> the scope and release gate, [PR #19](https://github.com/ethereum/desktop-wallet/pull/19)
> for the scaffolding direction, and
> [`kohaku-cli`](https://github.com/kassandraoftroy/kohaku-cli) as the **working prototype
> whose flows and command surface v0.1.0 follows**. What this doc adds is a dependency
> ordering, acceptance criteria, and an explicit account of what v0.1.0 does and does not
> cover. Everything here is meant to be argued with, re-cut, and re-ordered.
>
> Issues are mirrored to the
> [Account Interface project board](https://github.com/orgs/ethereum/projects/151) as they
> are opened.

## How this backlog is built

Following the conventions in [`README.md`](./README.md):

- **Dependency- and risk-ordered**, not ordered by appeal: load-bearing, hard-to-change
  foundations first.
- **Interface-first.** Issues tagged `interface` define a core API surface and are merged as
  an agreed stub before work fans out behind them.
- **Vertical slices.** Each issue is a user-observable outcome through core to UI, not a
  single-layer fragment. For v0.1.0 the UI is the CLI (EDW-006), so "user-observable"
  means "driveable from the CLI."
- **Acceptance criteria on every issue**, so "done" is checkable without the author
  adjudicating.
- **Issue IDs use the `EDW-###` convention** (Ethereum Desktop Wallet). IDs are stable
  identifiers, not positions: issues added later keep their number and sit wherever the
  dependencies put them, so EDW-021 and EDW-022 appear mid-document by design.

Every issue also inherits the **definition of done** in [`README.md`](./README.md), including
the second-reviewer requirement for anything touching keys, signing, storage, or the trust
boundary.

---

## Following the `kohaku-cli` prototype

`kohaku-cli` is a TypeScript terminal wallet that already does most of what v0.1.0 targets:
BIP-39 seed wallets encrypted on disk, HD public accounts with fresh-address derivation,
aggregated public and private balances, transfers, raw contract calls, and shield / unshield
against Tornado Cash, Railgun, and Privacy Pools through the `@kohaku-eth/*` plugins. It is a
**UX and flow prototype, not an implementation to port**: it is Node and TypeScript, the
desktop wallet is Rust, so what carries over is the model and the command surface.

**The protocol code underneath it is a different story, and the split matters.**
`ethereum/kohaku` is a mixed repo, and what exists in Rust is what a Rust wallet can reuse:

- **Railgun: Rust** (`crates/railgun`), and the npm package the prototype consumes is a wasm
  wrapper over it (`crates/railgun-ts` is a `cdylib` with `wasm-bindgen`).
- **Tornado Cash: Rust in progress** (`crates/tornadocash`, largely Robert's work), with
  circuit artifacts, an indexer, and merkle/prove benches already in place. **TypeScript is
  what is in production**, so the Rust implementation still needs parity verification against
  it, and there is real detail work left there.
- **Privacy Pools: TypeScript only** (`packages/privacy-pools`). Choosing it would mean
  writing the protocol and its proving path from scratch.
- **`userop-kit`** (ERC-4337 EntryPoint, UserOperations, smart accounts, bundler client) and
  the ZK primitives (`crypto`: Pedersen and BabyJubJub; `poseidon-rust`) are **Rust**.

This drives EDW-013 and open question 6.

> **Note on freshness.** `kohaku-cli` moves fast. As of `0.0.3` it has dropped React, Ink,
> and its TUI, moved from ethers to viem, and added Tor, stealth addresses, ENS name
> management, and network-traffic logging. Anyone reasoning about diffs against it should
> pull the latest and rebuild `node_modules` first; the observations below are current as of
> that version.

**Carry over:**

- **The Profile concept**, and more centrally than the prototype has it. In `kohaku-cli` the
  profile is a container; here it should be the thing the user actually drives, with
  address-level detail staying invisible (principle 9). This is the single biggest intended
  departure and it drives EDW-022.
- **Tor by default for every non-RPC HTTP egress.** The prototype now ships `tor-js` and
  routes through it. Whatever egress survives the audit in EDW-021 goes through Tor rather
  than direct.
- **Network-traffic visibility.** The prototype logs egress and ships a viewer for it. That
  turns principle 2 from an assertion into something a user and a reviewer can check.
- **The account model.** BIP-39 seed as the primary object, HD-derived public accounts,
  `next-fresh-address` to derive and persist the next one, and import-by-mnemonic that scans
  for used addresses to resume the account index. This is the model v0.1.0 should adopt.
- **Dry run by default.** `transfer`, `transact-raw`, `shield`, and `unshield` prepare and
  print the transaction; `--broadcast` is an explicit second step. This is a good default for
  a wallet of last resort and it should be ours.
- **`--non-interactive` with JSON output on every command**, so flows are scriptable and
  testable rather than only demonstrable.
- **Per-protocol scoped encrypted storage**, which matches the scoped-database shape already
  in PR #19.
- **Tail calls appended after a payout** (`--tail-calls`), which is the flashcall primitive in
  its simplest form and direct prior art for EDW-019.
- **Honest constraint surfacing.** The prototype states plainly that Tornado shields must be
  multiples of 0.1 ETH, that an unshield consumes exactly one note, and that large shields may
  need multiple unshields. That is principle 6 in practice.
- **Chain binding per wallet** (`--testnet`, RPC chain ID must match), which covers the
  vision's testnet requirement cleanly.

**Do NOT carry over:**

- **`export-private-key` printing raw key material to stdout**, and
  **`see-decrypted-storage`** decrypting wallet storage to stdout. These are prototype
  conveniences and direct violations of principles 1 and 11. If an export path is needed at
  all, it needs a different design.
- **Unaudited non-RPC egress.** The prototype reaches `public.pimlico.io` (4337 bundler),
  `fastrelay.xyz` (relayer), `api.0xbow.io` / `dw.0xbow.io` (Privacy Pools association data),
  a state-sync host, an artifact host, and a USD price feed. **Some of this is structural
  rather than sloppy**: a bundler or relayer is what makes private gas work at all. So the
  goal is not zero egress, it is a deliberate, minimal, Tor-routed, and documented set. That
  is EDW-021, and it is why the egress decision landed the way it did rather than by cutting
  the sponsor.
- **The dependency surface**, though less than it was: `0.0.3` dropped React and Ink and
  moved to viem, and now carries roughly seventeen runtime npm packages. Principle 4 still
  asks for materially less than that inside the trust boundary.

**Vocabulary note.** The prototype's model is directional: public accounts **shield** into a
private protocol and **unshield** back out. [`01-architecture.md`](./01-architecture.md)
generalizes this to a `Vault` trait where any vault can deposit to or withdraw from any other.
Ours is the more general abstraction, and EDW-015 is the issue that proves it. If shield and
unshield are the words users see, they should be added to
[`vocabulary.md`](./vocabulary.md) as the user-facing names for the directional case.

---

## v0.1.0 - the walking skeleton

**Target: 2026-09-30.**

v0.1.0 is a **thin vertical slice through the whole privacy thesis** rather than a complete
first layer. The point is to prove the [`01-architecture.md`](./01-architecture.md)
decomposition (Profile / Signer / Executor / Vault over a Provider and a Database) survives
contact with three genuinely different vault implementations and a dapp session, with
`kohaku-cli` as the reference for what the flows should feel like. Depth comes after.

### Release gate (from issue #15)

v0.1.0 ships when all of these work end to end:

- [ ] Create a new profile, or import an existing wallet / vault
- [ ] View ETH balance via the JSON-RPC provider
- [ ] Send some assets to an eth address
- [ ] Transfer between two vaults
- [ ] Interact with a dapp from a public / stable address
- [ ] Interact with a dapp from a private / ephemeral address
- [ ] Interact with a dapp via a flashcall
- [ ] Backend builds for target platforms

### Explicitly NOT in v0.1.0

Called out because each is authoritative MVP scope in [`00-vision.md`](./00-vision.md), and
silence would read as "delivered":

- **Conformance vectors and the published convention.** v0.1.0 derives from a seed
  (EDW-007), but the derivation convention is not yet frozen, documented as a normative
  spec, or backed by committed known-answer vectors that an independent implementation can
  reproduce. That is the deliverable principle 10 turns on, and it is the first thing after
  v0.1.0.
- **Private reads (principle 3).** A plain JSON-RPC provider lets the provider correlate
  every address in a profile, and private-state sync via chunked `eth_getLogs` makes that
  worse, not better. v0.1.0 accepts this; v0.2.0 does not.
- **Auto-mix-back / background mixing.** Manual only.
- **Fund classification and footgun guards (principles 7 and 8).** Not surfaced.
- **A graphical UI, and an interactive TUI.** The v0.1.0 surface is a flag-driven CLI. The
  next UI step after that is a real GUI, not a richer terminal; a TUI would be halfway to
  neither. This defers vision decision 5 without blocking core work.

**Egress limiting and Tor are now IN scope** (EDW-021), moved in during review. They were the
one principle-2 item where deferring would have meant shipping v0.1.0 with unaudited
third-party egress and then retrofitting, which is the harder order.

Per principle 6, the v0.1.0 CLI should say plainly that it is a development
preview and does not yet deliver the privacy properties in the vision, in the same spirit as
the prototype's own README notice.

---

## v0.1.0 issues

The five stages below are a reading order, not gates. Work in a later stage can start as soon
as its own `needs` are met; the `needs EDW-###` links are the real dependency structure.

### Stage 1 - Foundations

Nothing user-visible. Everything after this depends on it.

**EDW-001 - CI and a one-command build**
`infra` - blocks everything

Why: nothing currently enforces the workspace's deny-level lints, and `cargo test` does not
compile on a clean clone because `sol!` reads a gitignored Foundry artifact.

- [ ] A clean clone builds and tests with one documented command (a `just` recipe), which
      runs `forge build` before `cargo test`.
- [ ] CI runs `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test`,
      `forge build`, and `forge test` on every PR.
- [ ] Dependency audit (`cargo deny` or `cargo audit`) runs in CI, per principle 4.
- [ ] CI is green on Linux and macOS.

**EDW-002 - `SimpleDelegate` is tested and its deployment is verifiable**
`core` `security` - needs EDW-001

Why: the 7702 delegate gates every asset movement, and it currently ships with no Solidity
tests and a hardcoded implementation address that nothing verifies.

- [ ] Forge test suite covering: valid batch executes, nonce increments, replay is rejected,
      a non-owner signature is rejected, an inner revert propagates.
- [ ] `solc` and `evm_version` pinned in `foundry.toml`, and the pragma pinned, so bytecode
      is reproducible.
- [ ] A test derives the CREATE2 address from the deploy script and asserts it equals the
      constant used by the Rust impls (or the constant is removed until a real deployment
      exists).
- [ ] A decision is recorded on `fallback()`: without it, a delegated EOA cannot receive
      ERC-721/1155 safe-transfer callbacks or answer ERC-1271.

**EDW-003 - Secret material is encrypted at rest**
`core` `security` `interface` - needs EDW-001

Why: the scaffold writes raw signing keys through the `Database` trait, which has no
encryption seam. This is principle 5, and the shape is expensive to change later.

- [ ] The seam is decided and documented: does the `Database` impl encrypt, does a repository
      layer above it encrypt (as [`01-architecture.md`](./01-architecture.md) proposes), or do
      secrets leave `Database` entirely for a separate keystore object? Update the
      architecture doc with the answer.
- [ ] Argon2id (64 MiB / 3-pass) + XChaCha20-Poly1305 over a versioned blob; root of trust is
      the user's password, not an OS store.
- [ ] Per-object scoping, so each vault and executor gets an isolated keyspace (the prototype
      does this per protocol; PR #19's scoped database is the same shape).
- [ ] A test writes a profile, then scans the on-disk bytes and asserts no plaintext key
      material is present.
- [ ] Wrong password fails via the AEAD tag, cleanly, not via a panic.
- [ ] Every type holding secret material is zeroize-on-drop.

**EDW-004 - `Signer` trait and a local seed-backed signer**
`core` `interface` - needs EDW-003

Why: `Signer` is one of the three core objects in the architecture and the vocabulary, and it
is the only one with no implementation yet.

- [ ] `Signer` trait merged as specced: `id`, `public_key`, `sign_message`, `personal_sign`,
      `sign_typed_data`.
- [ ] A local impl whose key is loaded through EDW-003 and never returned by any public
      method (principle 1). No export-to-stdout path, unlike the prototype.
- [ ] Known-answer tests for `personal_sign` and `sign_typed_data`.

**EDW-005 - `EthereumProvider` seam and a configurable network**
`core` `interface` - needs EDW-001

Why: everything on-chain goes through this, and the vision requires switching networks
without recompiling.

- [ ] The seam is decided: use alloy's `Provider` directly, or define a minimal trait to
      avoid a hard alloy dependency in the facade. Record the choice and the reasoning.
- [ ] JSON-RPC impl. A light client (Helios) is explicitly out of scope for v0.1.0.
- [ ] Endpoint and chain are configurable at runtime; mainnet, a testnet, and a local devnet
      all work against the same binary, with the profile bound to a chain ID so a mismatched
      RPC is rejected rather than silently used.
- [ ] Chunked `eth_getLogs` with a configurable span, since private-state sync needs it and
      strict providers reject large ranges.

**EDW-021 - Every non-RPC egress is audited, minimal, and Tor-routed**
`core` `security` `infra` - needs EDW-005

Why: moved into v0.1.0 during review. Private gas requires a sponsor, so zero non-RPC egress
is not achievable (see Decisions taken in review), which makes a deliberate and observable egress set the
actual goal. Retrofitting this after v0.1.0 ships is the harder order.

- [ ] Every outbound host the app can contact is enumerated in one place, with what it sees
      about the user, and that list is part of the threat model rather than folklore.
- [ ] All non-RPC HTTP goes through **Tor by default**, following the prototype's `tor-js`
      approach, degrading with a clear error rather than silently falling back to direct.
- [ ] Anything that can come from RPC does, and anything that can be built locally is, per
      principle 2. No fiat price feed in v0.1.0.
- [ ] Egress is logged and inspectable by the user, following the prototype's
      network-traffic viewer, so principle 2 is checkable rather than asserted.
- [ ] A test fails the build on an unexpected outbound host.

### Stage 2 - A profile that holds and moves funds

**EDW-006 - CLI surface**
`ui` - needs EDW-001

Why: v0.1.0's UI is a flag-driven CLI, and it needs to exist early so every subsequent issue
can be a real vertical slice rather than a library change.

- [ ] **Flag-driven subcommands, no interactive TUI** (decided in review; the prototype has
      since dropped its own TUI).
- [ ] Unlock / lock, with the password never echoed or logged.
- [ ] `--non-interactive` with JSON output on every command that produces data.
- [ ] Every state-changing command dry-runs by default and requires an explicit
      `--broadcast`.
- [ ] The surface links only the facade crate and holds no secret material (principle 1).
- [ ] Prints a development-preview notice stating the privacy properties not yet delivered.

**EDW-007 - Seed, HD derivation, and a profile's public accounts**
`core` `interface` `security` - needs EDW-003, EDW-004

Why: the prototype's account model, and the foundation the User Namespace Convention is built
on. Everything downstream derives from this, so getting the derivation shape wrong is the
most expensive mistake available in v0.1.0.

- [ ] BIP-39 generation and import, with the mnemonic shown once on creation and never again.
- [ ] HD derivation of public accounts, persisted, with a "derive the next fresh account"
      operation.
- [ ] Import scans for used addresses and resumes the account index, so a restored profile
      does not re-issue addresses that already have activity.
- [ ] The derivation paths in use are written down in this repo, even though freezing them as
      a normative convention with conformance vectors is deliberately v0.2.0 work.
- [ ] The seed never leaves the core and is zeroize-on-drop.

**EDW-008 - Private-key vault and executor, hardened from the scaffold**
`core` `security` - needs EDW-002, EDW-003, EDW-007

Why: PR #19 landed these as a scaffold; v0.1.0 needs them trustworthy.

- [ ] Signing keys reach storage only through EDW-003.
- [ ] `Executor` reports success versus revert; a reverted batch is not reported as success.
- [ ] `Vault::deposit`'s `from` parameter is either honored or removed from the trait.
- [ ] Withdrawal authorizations carry an expiry, and their nonce is read in a way that two
      pending withdrawals cannot silently collide.
- [ ] No debug output in any send path (principle 11).

**EDW-009 - Create a new profile, or import one from a seed**
`core` `ui` - needs EDW-006, EDW-008

Why: the first line of the release gate, and the entry point to everything else.

- [ ] `profile create` generates a seed and produces a profile with one private-key vault,
      executor, and signer.
- [ ] `profile import` restores from a mnemonic.
- [ ] A profile created and never modified survives a restart (the scaffold currently only
      persists on the first vault add).
- [ ] Profile state round-trips through EDW-003 with no plaintext on disk.
- [ ] Listing profiles works without unlocking any of them.

**EDW-010 - View a profile's balance**
`core` `ui` - needs EDW-005, EDW-009

Why: second line of the release gate, and the first proof that `Profile` aggregating over
vaults works.

- [ ] Aggregate ETH total across every vault in the profile, plus a per-vault breakdown
      behind a verbose flag.
- [ ] ERC-20 balances for assets the user has opted into, per `vocabulary.md`, seeded from a
      locally-stored default list per chain rather than a fetched one.
- [ ] Balances come from the EDW-005 provider only, with no other network egress. Fiat
      valuation is out of scope for v0.1.0 precisely because it implies a price feed.

**EDW-011 - Send assets to an address**
`core` `ui` - needs EDW-010

Why: third line of the release gate.

- [ ] Send ETH or an ERC-20 from a chosen account to any address.
- [ ] Dry run prints the transaction; `--broadcast` signs and submits.
- [ ] Fee estimation is derived from chain state, not a hardcoded heuristic.
- [ ] Reports final inclusion and whether the transaction succeeded or reverted.
- [ ] Insufficient balance and insufficient gas fail with actionable errors, not panics.

### Stage 3 - More than one kind of vault

This is the stage where the `Vault` abstraction either holds up or does not. It is the
highest-risk part of v0.1.0 and the reason the abstraction is worth proving now. It is also
the stage with the most prototype coverage to lean on.

**EDW-012 - Stealth-address vault (ERC-5564)**
`core` - needs EDW-008

Why: stealth is the standard for direct transfers in the vision, and it is the first vault
whose balance is not simply one address's balance. The prototype added stealth support in
`0.0.3`, so there is now prior art here too.

- [ ] Meta-address generation, announcement scanning, and claiming.
- [ ] `balance` aggregates across discovered stealth addresses.
- [ ] `withdraw` produces calls that spend from the discovered addresses.
- [ ] Test vectors from ERC-5564 pass.

**EDW-013 - Shielded-pool vaults: Railgun and Tornado Cash**
`core` `research` - needs EDW-008

Why: the vault kind that proves privacy is real rather than architectural, and the largest
single integration in v0.1.0.

**Decided in review: both Railgun and Tornado Cash, and nothing else.** They are the two with
the most mature Rust support, they sit at opposite ends of the shielded-pool design space
(Privacy Pools sits between them), and shipping both means the wallet is built for a
multiple-mixer world from day one rather than retrofitted into one. That is a deliberate
scope increase over the one-protocol version of this issue; it is the main thing to weigh
against the date in open question 5.

- [ ] Shield and unshield end to end on a testnet, for **both** protocols.
- [ ] Tornado specifically: the Rust `crates/tornadocash` implementation is verified at
      parity with the TypeScript one that is in production, since TypeScript is what is
      battle-tested today and the Rust port still has detail work outstanding.
- [ ] The two protocols' very different constraint models (fixed denominations and
      single-note unshields for Tornado, arbitrary-amount UTXOs for Railgun) are both
      expressed through `AssetConstraint` without either one special-casing the `Vault`
      trait.
- [ ] `balance` reflects spendable notes; note secrets are stored via EDW-003, not in the
      protocol crate's own storage layer.
- [ ] Whatever constraints each protocol imposes (denomination multiples, one note per
      unshield, note-size caps) are stated plainly in the UI, following the prototype's
      example.
- [ ] The anonymity set is reported honestly and never overstated (principle 6).
- [ ] If a Kohaku crate is used: the dependency is pinned to a rev, its overlapping
      `database` and `provider` abstractions are adapted to our seams rather than leaking
      through, and the added dependency surface is reviewed against principle 4.

**EDW-014 - Unshield to an address that has never held gas**
`core` `research` `interface` - needs EDW-013

Why: a delinked recipient with no ETH cannot pay for its own withdrawal, so this is what makes
private receipt actually usable. The prototype solves it with a 7702 delegation plus a
paymaster or relayer, and PR #19's `Executor` currently has no notion of sponsored execution.

- [ ] The sponsorship path is decided and recorded: bundler and paymaster, a relayer, or a
      self-relay gas tank. Each has a different egress and trust profile; see EDW-021.
      Note that Kohaku's `userop-kit` gives us a Rust 4337 path (EntryPoint, UserOperations,
      smart accounts, a bundler client) essentially for free if we take the Railgun crate,
      since Railgun already depends on it. That makes the bundler-and-paymaster route the
      path of least resistance, which is precisely why it deserves a deliberate decision
      rather than an inherited one.
- [ ] The `Executor` seam covers sponsored execution without vault implementations knowing
      which sponsor is in use.
- [ ] A fresh account with zero balance receives an unshield end to end on a testnet.
- [ ] Whatever the sponsor sees about the user is documented in the threat model.

**EDW-015 - Move assets between any two vaults**
`core` `ui` - needs EDW-012, EDW-013

Why: fourth line of the release gate, and the real test of the two-step inter-vault design in
the architecture. This is where our abstraction goes beyond the prototype's directional
shield / unshield model, so it is the one most likely to expose a design flaw.

- [ ] Direct path: source vault supports the destination and returns a single call.
- [ ] Fallback path: withdraw to an ephemeral address, then deposit, executed atomically.
- [ ] Works for every pair among the private-key, stealth, and shielded vaults.
- [ ] No vault implementation imports or matches on another vault implementation's type.

**EDW-022 - The user operates on the Profile, not on scattered addresses**
`core` `ui` - needs EDW-014, EDW-015

Why: **this is the intended headline difference from the prototype.** In `kohaku-cli` the
only integrated flow is unshield-plus-tail-call; outside that narrow path the user is left to
work out their own address juggling by hand. The canonical broken case: an ERC-20 arrives at
a stealth address that holds no ETH, and shielding it means the user manually unshields gas
first, or hand-assembles an unshield-plus-tail-call. Principles 8 and 9 say the wallet should
do that, not the user.

- [ ] Funds sitting at an address that cannot pay for its own next action are detected, and
      the wallet composes the gas path itself rather than reporting a dead end.
- [ ] The ERC-20-at-a-stealth-address case works without the user naming an intermediate
      address or hand-ordering calls.
- [ ] Profile-level actions ("shield this", "send this") resolve which underlying accounts
      and sponsorship they need, rather than requiring the user to pick a source account.
- [ ] Where the wallet cannot do it automatically, it says so plainly rather than implying
      the funds are stuck or that more privacy was achieved than actually was (principle 6).

### Stage 4 - Dapps

The prototype has **no dapp support at all**. Its answer to "I want to use a frontend" is
`export-private-key`, and load that key into a browser wallet. So this stage is not just
unprototyped, it is a **deliberate departure** from how the prototype works, and open
question 3 asks whether it belongs in v0.1.0 at all. It carries the most schedule risk in the
milestone either way.

**EDW-016 - Raw contract calls from a profile account**
`core` `ui` - needs EDW-011

Why: the minimal "interact with a contract" primitive, and a stepping stone that de-risks the
dapp work. The prototype's `transact-raw` is the model.

- [ ] Submit one or more ordered calls with targets, calldata, and values from a chosen
      account.
- [ ] Every call is simulated before anything is broadcast.
- [ ] Dry run prints the payloads; `--broadcast` submits.

**EDW-017 - Connect to a dapp from a stable address**
`core` `ui` `interface` - needs EDW-016

Why: fifth line of the release gate, and it defines the dapp session seam.

- [ ] Session transport seam merged as a trait, with one impl. Which transport (OpenLV,
      WalletConnect) is an implementation choice behind it.
- [ ] A dapp can read chain state and request a signature or transaction.
- [ ] Every request is surfaced for explicit approval; nothing is auto-approved.

**EDW-018 - Connect to a dapp from a fresh, ephemeral address**
`core` `ui` - needs EDW-017, EDW-015

Why: sixth line of the release gate, and the first behavior that is actually private by
default.

- [ ] Connecting derives a fresh account by default, with the stable account opt-in.
- [ ] The fresh account is funded from mixed funds where possible.
- [ ] The UI shows which address a dapp sees and whether it is linked to the profile's public
      identity.

**EDW-019 - Flashcall: fund, interact, and defund atomically**
`core` `ui` - needs EDW-018

Why: seventh line of the release gate, and the strongest single demonstration that private by
default and generalistic can coexist. The prototype's `--tail-calls` on unshield is the
primitive in its simplest form.

- [ ] One transaction funds an ephemeral address, calls the target contract, and returns the
      remainder to a vault, per `vocabulary.md`.
- [ ] Partial failure reverts the whole batch, leaving no funds stranded.
- [ ] Demonstrated end to end against a real dapp contract on a testnet.

### Stage 5 - Ship it

**EDW-020 - Backend builds for the target platforms**
`infra` - needs EDW-001

Why: eighth line of the release gate.

- [ ] Builds for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, and
      `x86_64-pc-windows-msvc`, produced by CI.
- [ ] The core compiles for `wasm32-unknown-unknown` (the `inventory`-based factory registry
      needs checking here specifically).
- [ ] Tagged `v0.1.0` with artifacts and a release note listing what the release does and
      does not yet do.

---

## Candidate milestones after v0.1.0

The themes below are where each axis deepens once the skeleton stands up. They are **candidate
milestones, not a committed sequence**: several can run in parallel, more than one may land in
the same release, and their order should be re-cut after v0.1.0 shows which parts of the
abstraction were wrong. They are deliberately left unversioned until then.

- **Namespace and conformance.** Freeze the derivation convention from EDW-007, publish it as
  a normative document, and back it with committed known-answer vectors an independent
  implementation can reproduce. This is the deliverable the vision's success criteria turn on
  and it should be first.
- **Private reads and egress.** The private-read layer, a light client or local node path, and
  enforcement that nothing leaves the app except RPC. This is where the third-party services
  v0.1.0 tolerates get removed or justified.
- **Fund classification and footgun guards.** Linked / delinked / mixed as first-class state,
  surfaced at the moment of action.
- **Auto-mix-back.** Background mixing so returning funds to the pool is not a manual chore.
- **The graphical UI.** Whatever vision decision 5 settles on, built against the same facade
  the CLI uses.
- **Hardening and release.** Reproducible signed builds, external security review, threat
  model validation, and a conformance guide for other wallets.

---

## Decisions taken in review

Recorded here so the reasoning survives; each one is reflected in the issues above.

- **Shielded protocols: Railgun and Tornado Cash, and nothing else** (EDW-013). Most mature
  Rust support, opposite ends of the shielded-pool design space, and building for a
  multiple-mixer world from day one rather than retrofitting. Privacy Pools sits between the
  two and adds no new design pressure for the cost.
- **Private gas wins over strict only-RPC egress** (EDW-014, EDW-021). Zero non-RPC egress is
  not compatible with an unfunded address being able to receive privately. The goal becomes a
  minimal, documented, Tor-routed egress set rather than none, and egress limiting moves into
  v0.1.0 scope rather than being deferred.
- **Flag-driven CLI, no TUI** (EDW-006). The prototype has since dropped its own TUI. The
  next UI step after a working CLI is a real GUI; a TUI is halfway to neither.
- **The Profile is the thing the user drives** (EDW-022), not a container the user reaches
  through. This is the intended headline difference from the prototype.

## Open questions for the team

1. **Milestone axis.** This doc treats a **milestone as a release** (v0.1.0, v0.2.0), taking
   the release numbering from issue #15 and retiring the earlier `M0`-`M5` sketch. That makes
   a milestone here the same thing GitHub means by one, so issues map straight onto the
   project board. Within a milestone, work is grouped into numbered **stages**, which are a
   reading order rather than gates. `README.md` and `01-architecture.md` have been updated to
   match; if the team prefers the old `M<n>` axis, all three need reverting together.
2. **Does a Profile enshrine one mixer?** Proposal on the table: a Profile picks its shielded
   protocol at initialization, so there are `TornadoProfile` and `RailgunProfile` types.
   Balances in the other protocol are still discovered and displayed, but the Profile does
   not operate on them. The argument for is that the integrated profile-level flows in
   EDW-022 (unshield-interact-reshield, stealth-address gas handling, private gas) are much
   easier to make genuinely good when they can assume one protocol's semantics, and that the
   prototype only avoids this because it has no profile-level UX to speak of. The argument
   against is that it puts a protocol choice in front of the user at the moment they know
   least, and that a Profile spanning both is the more honest model of "these are your
   funds." **This is the biggest open design question in the document** and it directly
   shapes EDW-013, EDW-015, and EDW-022.
3. **Do dapp interactions belong in v0.1.0 at all?** The release gate in #15 includes three
   dapp lines, but the prototype's answer to using a frontend is `export-private-key` into a
   browser wallet, so stage 4 is a real departure rather than a port. Two sub-questions: is a
   first-party dapp connection in scope for the first release, and do we carry over
   `transact-raw` (EDW-016) as the minimal contract-interaction primitive, or is that also a
   frontend concern? Note this interacts with question 5.
4. **Does the vision's fixed-pool language survive?** [`00-vision.md`](./00-vision.md) defers
   the dust problem with "assume a small fixed pool (~0.01 ETH)", which is Tornado-shaped.
   Now that Railgun ships alongside Tornado, the wallet spans both a fixed-denomination and
   an arbitrary-amount UTXO model, and that sentence needs rewriting rather than deleting.
5. **Is the target realistic, and where does it give?** Stages 2 and 3 have substantial
   prototype coverage, which is the argument that 2026-09-30 is achievable. Working against
   it: EDW-013 now covers two protocols instead of one, EDW-021 and EDW-022 were added during
   review, and stage 4 has no prototype coverage at all. If something has to give, cut stage
   4 first (see question 3), rather than thinning stage 3.
6. **Kohaku crate dependency posture.** Taking `crates/railgun`, `crates/tornadocash`, and
   `userop-kit` means pinning revs of unstable, git-only `0.1.0` APIs and tying our cadence to
   theirs, and for Tornado specifically the Rust implementation is not yet verified at parity
   with the TypeScript one in production. Worth an explicit position on how we track that.
