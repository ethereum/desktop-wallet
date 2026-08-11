# 02 - Milestone Backlog

> **Status: DRAFT** (circulating for team review).
>
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
  single-layer fragment. For v0.1.0 the UI is the terminal surface (EDW-006), so
  "user-observable" means "driveable from the terminal."
- **Acceptance criteria on every issue**, so "done" is checkable without the author
  adjudicating.
- **Issue IDs use the `EDW-###` convention** (Ethereum Desktop Wallet).

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

**The protocol code underneath it is a different story, and the split matters.** Kohaku is a
mixed repo. Railgun is implemented in **Rust** (`crates/railgun`), and the npm package the
prototype consumes is a wasm wrapper over it (`crates/railgun-ts` is a `cdylib` with
`wasm-bindgen`). Tornado Cash and Privacy Pools exist **only as TypeScript packages**.
`userop-kit` (ERC-4337 EntryPoint, UserOperations, smart accounts, bundler client) and the ZK
primitives (`crypto`: Pedersen and BabyJubJub; `poseidon-rust`) are also Rust. So a Rust
wallet can reuse Railgun and the 4337 path directly, and would have to build a Tornado or
Privacy Pools integration from scratch, including its proving path. This drives EDW-013 and
open question 2.

**Carry over:**

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
- **Non-RPC egress.** The prototype reaches `public.pimlico.io` (4337 bundler),
  `fastrelay.xyz` (relayer), `api.0xbow.io` / `dw.0xbow.io` (Privacy Pools association data),
  and `saga.fatsolutions.xyz` (sync), plus a USD price feed. That is five third-party
  services with visibility into user activity, against principle 2. See open question 4:
  some of this is not sloppiness, it is structural, and we need a position on it.
- **The dependency surface.** Roughly fourteen runtime npm packages including React. Whatever
  the UI stack decision, principle 4 asks for materially less than this inside the trust
  boundary.

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
- **Egress limiting and Tor (principle 2).** Not addressed, and see open question 4.
- **Auto-mix-back / background mixing.** Manual only.
- **Fund classification and footgun guards (principles 7 and 8).** Not surfaced.
- **A graphical UI.** The v0.1.0 surface is a terminal UI, which also defers vision decision
  5 without blocking core work. See open question 3.

Per principle 6, the v0.1.0 terminal surface should say plainly that it is a development
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

### Stage 2 - A profile that holds and moves funds

**EDW-006 - Terminal surface**
`ui` - needs EDW-001

Why: v0.1.0's UI is the terminal, and it needs to exist early so every subsequent issue can
be a real vertical slice rather than a library change.

- [ ] Scope decided first: flag-driven subcommands only, or a full interactive TUI as in the
      prototype. See open question 3.
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
whose balance is not simply one address's balance. Note this is the one vault kind the
prototype does **not** cover.

- [ ] Meta-address generation, announcement scanning, and claiming.
- [ ] `balance` aggregates across discovered stealth addresses.
- [ ] `withdraw` produces calls that spend from the discovered addresses.
- [ ] Test vectors from ERC-5564 pass.

**EDW-013 - Shielded-pool vault**
`core` `research` - needs EDW-008

Why: the vault kind that proves privacy is real rather than architectural, and the largest
single integration in v0.1.0.

- [ ] **Which protocol ships in v0.1.0 is decided first, and it is a cost decision, not a
      taste one.** Railgun has a Rust implementation in Kohaku that already handles note
      management, merkle sync, and Groth16 proving (`ark-groth16` / `ark-circom`). Tornado
      Cash and Privacy Pools are TypeScript-only there, so either would mean writing the
      protocol and its proving path from scratch in Rust. One protocol is enough to prove the
      `Vault` abstraction. See open question 2.
- [ ] Shield and unshield end to end on a testnet.
- [ ] `balance` reflects spendable notes; note secrets are stored via EDW-003, not in the
      protocol crate's own storage layer.
- [ ] Whatever constraints the chosen protocol imposes (denomination multiples, one note per
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
      self-relay gas tank. Each has a different egress and trust profile; see open question 4.
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

### Stage 4 - Dapps

The prototype has **no dapp support at all**, so this stage has no prior art to lean on and
carries the most schedule risk in v0.1.0. See open question 5.

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
  the terminal surface uses.
- **Hardening and release.** Reproducible signed builds, external security review, threat
  model validation, and a conformance guide for other wallets.

---

## Open questions for the team

1. **Milestone axis.** This doc treats a **milestone as a release** (v0.1.0, v0.2.0), taking
   the release numbering from issue #15 and retiring the earlier `M0`-`M5` sketch. That makes
   a milestone here the same thing GitHub means by one, so issues map straight onto the
   project board. Within a milestone, work is grouped into numbered **stages**, which are a
   reading order rather than gates. `README.md` and `01-architecture.md` have been updated to
   match; if the team prefers the old `M<n>` axis, all three need reverting together.
2. **Which shielded protocol for v0.1.0?** The prototype supports Tornado Cash, Railgun, and
   Privacy Pools, but that parity does not carry into Rust. Only **Railgun** has a Rust
   implementation in Kohaku, and it is the substantive one: notes, merkle sync, and Groth16
   proving via arkworks, with the npm package built as wasm on top of it. Tornado Cash and
   Privacy Pools are TypeScript-only, so choosing either means implementing the protocol and
   its proving path from scratch. The recommendation is Railgun for v0.1.0 unless there is a
   reason not to, with the others deferred, where they double as the real test of whether the
   `Vault` abstraction is genuinely protocol-agnostic.

   Two consequences to weigh. Depending on Kohaku's crates means pinning a rev of an
   unstable, git-only `0.1.0` API and tying our cadence to theirs. And Railgun is UTXO-based
   with arbitrary amounts rather than fixed denominations, so choosing it largely dissolves
   the dust and denomination problem that [`00-vision.md`](./00-vision.md) defers with its
   "assume a small fixed pool (~0.01 ETH)" language. That assumption is Tornado-shaped and
   would need updating in the vision if Railgun wins.

   Separately, shipping any specific pool integration in an EF-published reference
   implementation is a call worth making deliberately rather than inheriting.
3. **Flag-driven CLI, or a full TUI?** Issue #15 says "CLI UI surface." The prototype has
   both: scriptable subcommands and an interactive Ink-based TUI with screens and panes. The
   second is materially more work and pulls in a UI framework, which touches vision decision
   5. Worth settling before EDW-006 starts.
4. **Only-RPC egress versus private gas: which gives?** Principle 2 says the only outbound
   calls are RPC. But the prototype reaches a bundler, a relayer, an association-set
   provider, and a sync service, and at least the sponsorship dependency is structural: an
   unfunded fresh address cannot pay for its own withdrawal without someone else submitting
   it. EDW-014 cannot be specified until the team decides whether we accept a third-party
   sponsor with a documented trust profile, build a self-relay gas tank, or narrow what
   principle 2 claims. This is the most load-bearing open question in the document.
5. **Is the target realistic, and where does it give?** The prototype substantially de-risks
   stages 2 and 3, which is the strongest argument that 2026-09-30 is achievable. Stage 4 has
   no prototype coverage at all. If something has to give, cut stage 4 to EDW-016 and EDW-017
   and move the two private-dapp flows to the next milestone, rather than thinning stage 3.
