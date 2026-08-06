# 00 - Vision & Principles

> The stable core of the spec. Everything else serves this. When a lower-level decision is
> ambiguous, resolve it against the **Principles** and **Threat model** below.
>
> **Status: DRAFT** (circulating for team review). Treat this as a starting position to pull apart, not settled doctrine. Points
> flagged _(open for review)_ are the ones most likely to change during review.

## What we are building

The **Ethereum Foundation Account Interface team** is building the **Ethereum Desktop
Wallet**, a **self-sovereign, privacy-focused Ethereum L1 wallet**. It has two complementary
deliverables: a **written specification** that pins down the normative conventions
(derivation paths, the namespace model, privacy behaviors) precisely enough that any team
could build a conforming wallet from the document alone, and a **reference implementation**,
the working code that proves those conventions out and stands as the canonical example.

The goal is not just to ship a wallet: it is to be a **wallet of last resort**, in two
senses at once. It never _compromises_ the user: whatever the rest of the ecosystem does,
there is always a wallet that upholds self-sovereignty and private-by-default. And it never
_abandons_ the user: with maximally limited dependencies (e.g. an Ethereum RPC) and no company or server in the trust path, it
keeps working even if the surrounding services disappear, the wallet you can always fall
back to. From that follows the mission to be **the reference implementation other wallets
conform to**, demonstrating that "private by default" _and_ generalistic is possible, and
standardizing the conventions (especially the User Namespace Convention) so the wider
ecosystem can adopt them.

We intend to raise the bar in two concrete ways:

- **Conformance is a product, not a side effect.** The derivation conventions, namespace
  model, and privacy behaviors are specified precisely enough that a _different_ team can
  build a conforming wallet and reconstruct the identical set of seed-derived objects from the
  same seed.
- **Legibility and neutrality matter.** Other wallets will read this code and this spec as
  the canonical example. It must be clean, documented, and free of choices that only make
  sense for us.

The product target for **v1 is the full privacy thesis** below, not a standard EOA wallet.
The work is **milestone-phased**: each milestone ships something usable and de-risks the
next. The detailed backlog is deferred until this vision and the architecture are agreed
(see [`02-backlog.md`](./02-backlog.md)).

## Who it's for

**Primary user (v1):** the **privacy-native Ethereum user**: comfortable with self-custody,
understands _why_ they'd want stealth addresses and mixing, and is currently underserved by
wallets that bolt privacy on as an afterthought.

**The thing we are proving:** that a wallet can be **private by default** _and_ still
generalistic and legible, usable by someone who does not want to learn the machinery. v1
makes the safe path the easy path and abstracts address-level detail away; it does not yet
try to serve a fully mainstream user, but every decision is made so it _could_ grow there.

## The MVP scope (authoritative)

This is the north star for every scope call. It is deliberately narrow.

**What we ARE focused on:**

1. **User Namespace Convention.** The user manages **Profiles**: a user-facing collection of
   **signers**, **executors**, and **vaults** (see `vocabulary.md`). **Most users have exactly
   one**, but many can attach to a single seed. The user interacts and thinks at the **Profile
   level**; the individual objects and their addresses are abstracted away as "low level." The
   seed-derived objects in a Profile follow a standardized derivation convention so the set is
   portable and reproducible; externally-sourced objects (hardware, remote) are simply added.
2. **Network Level Privacy: Limit HTTP outside RPC** Maximally limit network egress the app makes outside of Ethereum RPC Calls. Policy that any data which _could_ come directly from Ethereum RPC rather than alternative sources (indexers, price feeds etc) _should_. Policy that anything which _could_ be locally stored and built rather than fetched over http, _should_ be. Use of Tor by default for any egress. Stringent analysis of privacy implications of all network traffic. Critically: user funds and financial activity should not be deanonymized by outside observer, even with subpoena power over third party servers.
3. **Stealth addresses as the standard for direct transfers.** ERC-5564 stealth is an
   embedded, common default for person-to-person transfers, not an exotic opt-in.
4. **Private writes for dapp interactions.** By default you connect to a dapp with a **fresh
   address** and spend **mixed funds**: funds come out of the mixer to do the interaction
   wherever possible.
5. **Funds return to the mixer.** Unmixed funds go back into the mixer wherever possible.
   consider **auto-mixing / background mixing** so this is not a manual chore.
6. **Simple, legible, clean, convenient-enough UX.**
7. **Help the user avoid address-linking footguns.**

**What we are NOT focused on:**

- **No networks outside L1.** Zero chain interop, zero other networks. Not a concern _at
  all_. (Only support to build a `test` version of the app to use against testnets/devnets)
- **No hardware wallets** yet.
- **No recovery mechanism** yet, and **no multisig / multi-factor smart accounts** yet.
- **Dapp interaction UX only "good enough."** A generalistic dapp flow exists in its
  _minimal possible_ form: enough to demonstrate that "private by default" _and_
  generalistic is possible. Not a polished dapp browser.
- **Not solving Tornado's "dust" problem.** Left for later. Assume a small fixed pool
  (consider **0.01 ETH**) and don't design the fine-grained gas/dust economics now.

## Threat model (stated explicitly)

Every privacy and security claim is judged against this. We defend against:

- **Chain-analysis / de-anonymization:** an adversary correlating on-chain activity to link
  a user's delinked branches back to their public identity. _The core threat the privacy
  model exists to counter._
- **The RPC / chain-data provider:** must not be trusted for correctness and _should_ not be trusted to escrow user privacy (though RPC privacy in MVP is technically out-of-scope of the wallet itself, instead companion software like a full node or other solutions are simply available and work seamlessly with the wallet)
- **A passive network observer:** should not be able to trivially map wallet traffic to
  identities. This includes a passive network observer with power to subpoena third party services for logs and network metadata.
- **Local malware reading disk at rest:** no plaintext secrets on disk; strong KDF + AEAD;
  minimize secret lifetime in memory.
- **Supply-chain compromise:** minimized by keeping the dependency set small and auditable;
  further mitigated by dependency review and reproducible builds. _(How far to take this,
  e.g. all-Rust/no-npm, is an open UI-stack decision; see principle 4 and the decisions
  list.)_

We explicitly **do not** (in v1) defend against:

- A **fully compromised host** with a live keylogger / memory scraper while unlocked (the
  future hardware-signer path is the mitigation).
- **Global passive network adversaries** correlating timing across the whole internet.
- **Coerced disclosure** of the user's password/seed.

> Keep this list current. A feature that changes what we defend against updates this section
> in the same PR.

## Principles & constraints (non-negotiable)

The rules that let any contributor make a hundred small decisions the way the team would.
Violating one is a blocking review comment.

1. **Secret material never leaves the core.** Keys, seed, and derived private material live
   in `wallet-core`; the UI requests operations and receives _results_, never raw secrets.
2. **Limit egress.** The app's only outbound network calls are RPC and any _wholly necessary_ or _wholly privacy beningn_ endpoints. Telemetry/analytics/crash-reporting are principle violations, not features.
3. **Private reads.** Chain reads are trust-minimized _and_ structured so the RPC provider
   cannot trivially correlate a user's addresses.
4. **Self-sovereign supply chain.** Keep the dependency set minimal and auditable. Root of
   trust is the **user's password**, not an OS vendor's store; the OS keychain is a
   convenience layer whose absence degrades gracefully. _(Open for review: one option is all-Rust with no npm, while the new repo's dev shell
   provisions a Node/pnpm/Chromium
   toolchain, so the exact UI stack, and whether any JS tooling is acceptable inside the
   trust boundary, is an architecture decision the team should confirm. See
   [`01-architecture.md`](./01-architecture.md).)_
5. **No plaintext secrets at rest, ever.** Strong KDF + AEAD; zeroize on drop; minimize
   secret lifetime in memory.
6. **Honest privacy signaling.** The UI must **never imply more privacy than is delivered.**
   Surface the real state: is this address linked to my identity? how big is this pool's
   anonymity set? Overstating privacy is a _security bug_.
7. **Fund classification is first-class.** The wallet knows and shows whether funds are
   identity-linked / delinked / mixed; this powers the mix-back nudge and the footgun
   guards.
8. **Make the safe path the easy path.** Surface the public/private boundary at the moment
   of action and guard the known footguns (funding a stealth/fresh address from the identity
   anchor; withdrawing a mixer note to a public address in the same Profile; reusing a
   "fresh" address).
9. **Think in Profiles, not addresses.** The user operates at the Profile level; the signers,
   executors, and vaults it aggregates, and their address derivation, are an invisible default.
   Fresh addresses and stealth are machinery the user should rarely have to see.
10. **Interop / conformance obligation.** Another conforming wallet must reconstruct the
    **identical seed-derived objects** from the same seed. The convention covers the
    seed-derived subset of a Profile (not externally-sourced objects). As the reference
    implementation, our derivation conventions are a _standard_; bit-for-bit correctness is a
    hard requirement.
11. **Data minimization.** No telemetry, no analytics, no server-side crash reporting, no
    secrets in logs. Ever.
12. **Open, documented, and canonical.** Public source plus a written spec of the
    derivation/namespace conventions and design rationale, legible enough that the ecosystem
    can review, adopt, and build conforming wallets.

## In scope for v1

The capabilities in scope, phased across milestones. This lists _what_ is in v1; _how_ each
is built (protocols, dependencies, mechanisms) lives in the architecture and backlog, not
here. Detailed backlog forthcoming.

- **Foundation:** a formalized core API, convenient re-unlock without re-entering the
  password every session, transaction history, contract-call sends, accurate fee estimation
  and inclusion confirmation, robust error handling, CI + tests, and packaged/signed builds.
- **Profiles / User Namespace UX:** aggregated per-Profile view, invisible fresh-address
  generation, the public/private boundary surfaced at signing, fund classification, and
  correlation guards. Address-level detail abstracted away.
- **Stealth addresses:** the default path for direct transfers, so a payment is not linkable
  to the recipient's public identity on-chain.
- **Shielded pools & private gas:** deposit to and withdraw from a shielded pool, with the
  withdrawal's gas funded privately so a delinked recipient never needs pre-funded gas, plus
  **auto-mix-back / background mixing** so unmixed funds return to the pool.
- **Private dapp interactions (minimal):** connect to a dapp with a fresh address, spend
  mixed funds (the smallest flow that demonstrates "private by default _and_ generalistic").
- **Safety basics** that "not focused on recovery" must not drop: seed-backup flow at
  creation, encrypted-at-rest storage, lock/unlock (ideally auto-lock).
- **Network abstraction:** the wallet can switch between different RPC providers and networks (mainnet, testnets, local dev) without recompiling the app.
- **Cross-platform compatibility:** The backend api supports planned future cross-platform builds:
  - `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, `aarch64-apple-ios`, `aarch64-linux-android`, `wasm32-unknown-unknown`.  
  - If possible support `wasm32-wasip1`, `wasm32-wasip1-threads`, `aarch64-unknown-linux-gnu`, `i686-unknown-linux-gnu`, `aarch64-pc-windows-msvc`

## Out of scope for v1 (explicit non-goals)

- **Any network beyond Ethereum L1.** No multi-chain interop or l2-specific support. Assume the wallet is connected to a single L1-like network at a time.
- **Hardware-signer integration.** Architecturally provided for (the signer seam), not built.
- **Recovery: social recovery / multisig / multi-factor smart accounts.** Deferred, but "no
  recovery" must never read as "you can lose everything" (the basics above still ship).
- **A polished dapp browser.** Dapp UX is "good enough," minimal by design.
- **Solving Tornado's dust problem.** Deferred; assume a small fixed pool (~0.01 ETH).
- **Frontends targeting mobile / web / browser-extension.** V1 is desktop-only.
- **A webview-free native UI renderer.** A possible future upgrade; not a v1 concern.
- **DEX/swap/bridge, NFT galleries, fiat on-ramp.** Not this product.
- **EIP-8141 (Frame Transactions) dependence.** Forward-looking only; nothing in v1 relies
  on unshipped protocol changes.

## Success criteria

v1 is successful when:

- A privacy-native user creates/restores a wallet, operates one or more **Profiles** without
  thinking about addresses, sends directly via **stealth** by default, connects to a dapp
  with a fresh address spending **mixed funds**, and has unmixed funds flow **back into the
  mixer**, all **without accidentally linking private branches to their identity**, because
  the UI made the boundary visible and guarded the footguns.
- The app makes **highly limited network calls that do not risk user fund and activity privacy**, never writes a plaintext secret to disk, never
  hands a secret to the view layer, and never overstates the privacy actually achieved.
- An **independent implementation reconstructs the identical seed-derived objects** from a
  test seed (conformance), proving the convention is a real standard.
- The project is public, documented, and legible enough to serve as the ecosystem's
  reference implementation.
- A security review of the core (keys, vault, derivation, signing, trust boundary) is
  complete with findings resolved.

## Decisions the team needs a position on (not implementation)

Policy/strategy calls to decide deliberately rather than inherit:

1. **Confirm the primary user** (privacy-native now; how far toward mainstream "private by
   default" does v1 reach?).
2. **Deterministic derivation of shielded-note secrets from the master seed** trades
   compartmentalization for recoverability/portability. Upside: no per-protocol backup;
   downside: seed compromise exposes shielded history, and deterministic derivation can weaken
   the "fresh, unrelated secret" property some pool designs assume.
3. **Profile terminology.** "Profile" vs "identity" vs "wallet" as the user-facing word: pick
   one and use it everywhere (this spec uses **Profile** provisionally).
4. **How much in-context privacy education is mandatory** for a first-time user to operate
   stealth + mixing _safely_: "simple UX" ≠ "self-explanatory privacy."
5. **UI stack & supply-chain posture.** One option is all-Rust (Dioxus) with no npm; the
   new repo's dev shell provisions a Node/pnpm/Chromium toolchain. Decide the UI stack
   (pure-Rust vs. a web-based UI) and, with it, how strict the "minimal, auditable supply
   chain" principle is. This is the biggest open assumption to confirm or
   replace. See [`01-architecture.md`](./01-architecture.md).
