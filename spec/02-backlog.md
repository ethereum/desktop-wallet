# 02 - Milestone Backlog

> **Status: IN PROGRESS** (intentionally deferred). 🚧
>
> The detailed backlog is **not filled in yet, on purpose.** It is the most detailed and
> assumption-heavy layer of the spec, and it should flow _from_ an agreed vision and
> architecture, not get ahead of them. We are circulating [`00-vision.md`](./00-vision.md)
> and [`01-architecture.md`](./01-architecture.md) for team review **first**; once those are
> settled, the concrete issues land here.
>
> This file is a placeholder so the shape is legible and so cross-references from the other
> docs (the `M0`…`M5` milestone tags) resolve. Treat everything below as a **provisional
> sketch**, not a commitment.

## How the backlog will be built (approach)

When we fill this in, it will follow the conventions in [`README.md`](./README.md):

- **Dependency- and risk-ordered**, not ordered by appeal: load-bearing, hard-to-change
  foundations first.
- **Interface-first.** Each milestone opens with the `wallet-core` API surface it needs,
  agreed and merged as a stub, so UI and core work can proceed in parallel behind it.
- **Vertical slices.** Each issue is a user-observable outcome through core→UI, not a
  single-layer fragment.
- **Acceptance criteria on every issue**, so "done" is checkable without the author
  adjudicating.
- **Issue IDs use the `EDW-###` convention** (Ethereum Desktop Wallet).

## Provisional milestone themes (sketch, subject to change)

These names exist so the milestone tags elsewhere in the spec are meaningful. Scope per
milestone is decided _after_ the vision/architecture review.

- **M0 - Foundation hardening.** Freeze the core API; CI + dependency-audit; conformance
  test vectors; OS-keychain convenience layer; complete the send path (verified fees,
  inclusion confirmation, contract calls); transaction history.
- **M1 - Profiles, namespace UX & private reads.** Per-Profile view with address detail
  abstracted away; fund classification; boundary flags and correlation guards at signing;
  the private-read layer; the public derivation/namespace convention doc.
- **M2 - Stealth addresses (ERC-5564).** Stealth as the default for direct transfers:
  derive, announce, scan, claim.
- **M3 - Shielded pools, private gas & auto-mix-back.** A shielded pool via Kohaku; the
  private-gas strategy (self-relay gas tank → permissionless fee market); background
  mix-back.
- **M4 - Private dapp interactions (minimal).** Connect fresh, spend mixed funds, the
  smallest generalistic dapp flow that proves "private by default _and_ generalistic."
- **M5 - Hardening, audit & release.** Reproducible signed builds; internal + external
  security review; threat-model validation; user docs; a conformance guide for other wallets.

---

_Next step: once the vision and architecture reviews converge, expand M0 into concrete
`EDW-###` issues with acceptance criteria and dependency links, and open them on the project board._
