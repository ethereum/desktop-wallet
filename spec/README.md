# Ethereum Desktop Wallet - Specification

This directory is the source of truth for **what we are building and how work is
organized**. It exists so the **EF Account Interface team** can build, in parallel and
without any one author becoming a bottleneck, the **Ethereum Desktop Wallet**: a
privacy-focused Ethereum L1 wallet that serves as the ecosystem's **reference
implementation**, the spec _and_ the working example other wallets can conform to.

> **Status:** all three documents are **drafts circulating for team review**. Nothing here is
> final; that's the point of the review.

If you are new here, read in this order:

1. **[`00-vision.md`](./00-vision.md):** what the wallet is, who it's for, the threat
   model, and the principles every decision is judged against. Read once; re-read when a
   decision "feels" ambiguous; the answer is usually a principle here.
2. **[`01-architecture.md`](./01-architecture.md):** the system decomposition and, most
   importantly, the **contracts between components** (the `wallet-core` public API). This
   is what lets UI and core work proceed in parallel against a shared interface.
3. **[`02-backlog.md`](./02-backlog.md):** the work, broken into milestones and
   dependency-ordered issues with acceptance criteria. **A milestone is a release** (v0.1.0,
   v0.2.0), so it means the same thing here that it means on the project board.

---

## How this spec is layered

The spec is deliberately layered from stable-and-small at the top to detailed-and-growing
at the bottom. Higher layers change rarely; lower layers churn.

| Layer                        | Doc                                                    | Changes      | Owned by                               |
| ---------------------------- | ------------------------------------------------------ | ------------ | -------------------------------------- |
| **Vision / principles**      | `00-vision.md`                                         | Rarely       | Whole team, decided together           |
| **Architecture / contracts** | `01-architecture.md`                                   | Occasionally | Whoever owns the core; reviewed by all |
| **Feature specs**            | one file per feature under `features/`                 | Per feature  | The dev/pair who owns the feature      |
| **Issues / tasks**           | GitHub Issues (`EDW-###`), seeded from `02-backlog.md` | Constantly   | Individual devs                        |

**The rule that makes parallel work possible: interfaces before implementations.** Before
work fans out on any feature, the `wallet-core` API surface it needs (the function/trait
signatures in `01-architecture.md`) is agreed and merged as a stub. Then a UI dev can build
against the stub while another dev fills it in. Nailing the seam allows a serial project
to become a parallel one if the feature requires it.

---

## Writing a feature spec

When an issue is bigger than "obvious from the title," write a short feature spec before
coding. Put it in `spec/features/<short-name>.md`. Keep it to one page. Template:

```markdown
# Feature: <name>

**Status:** draft | agreed | in-progress | shipped
**Owner:** <name>
**Milestone:** v<x.y.z>
**Related:** links to issues, related specs in this directory, prior art

## Problem

What user-facing or security problem does this solve? Who has it? (Anchor to a
principle or threat in 00-vision.md; if you can't, question whether it's in scope.)

## Behavior

What the user sees and does. Happy path first, then the important variations.

## Non-goals

What this explicitly does NOT do, so reviewers don't expect it.

## Core / UI contract

The wallet-core API this needs (signatures). New types. What crosses the trust
boundary and what must never cross it.

## Acceptance criteria

- [ ] Concrete, checkable statements. "Done" = all boxes ticked.
- [ ] Include the security-relevant ones explicitly (e.g. "no secret material
      reaches the view layer"; "wrong password fails via AEAD tag, not a panic").

## Security considerations

Threats this feature opens or closes (ref. the threat model). What an auditor
should look at. Any privacy-signaling implication.

## Test plan

Unit tests in wallet-core; what must be verified live in the GUI.
```

The **acceptance criteria** section is the most important part: it's what lets a feature
be marked "done" without the author adjudicating, and it's what a reviewer checks against.

---

## Issue conventions

Issues are seeded from `02-backlog.md` and assigned to a release milestone. Use the
**`EDW-###`** ID convention (Ethereum Desktop Wallet). A good issue:

- **Title** is a user-observable outcome or a concrete deliverable, not a fragment.
  Good: "Send to an ENS name." Bad: "ENS resolver util."
- Has a one-line **why**, **acceptance criteria**, and the **API surface** it touches.
- Is **sized** to land in one PR where possible. If it can't, it's an epic: split it and
  make the interface-defining sub-issue first.

### Labels

- **Area:** `core` (wallet-core, security-critical), `ui` (the view layer),
  `infra` (CI, build, packaging, release), `docs`, `research` (design not yet settled).
- **Type:** `feature`, `bug`, `security`, `interface` (defines/changes a core API; review
  bar is highest), `spike` (timeboxed investigation, output is a decision not shipping code).
- **Dependency:** use GitHub's "blocked by" / task-list links. Once the backlog lands it
  will be dependency-ordered; preserve that when you file.
- **Onboarding:** `good-first-issue` for well-scoped, low-blast-radius work.

### Definition of done (applies to every `core`/`ui` issue)

1. Acceptance criteria all met.
2. `cargo build` clean on the whole workspace; `cargo clippy` clean.
3. Tests added/updated; `wallet-core` logic covered by unit tests.
4. For anything touching keys, signing, storage, or the trust boundary: a second person
   reviews specifically for secret handling (see the security-review note in
   `01-architecture.md`).
5. If behavior is user-visible, it's been driven live in the running app (not just
   unit-tested).
6. Docs updated if the change affects the public API or user flows.

---

## The most common first-timer mistakes this structure prevents

- **Splitting by layer instead of by outcome.** "You do all UI, you do all crypto" creates
  constant blocking. Issues here are vertical slices through core→UI that each deliver one
  observable behavior.
- **Thin tickets that require the author to explain them.** Acceptance criteria fix this.
- **Building the fun stuff before the load-bearing stuff.** The backlog will be ordered by
  dependency and risk: the scary, hard-to-change foundations (secret storage, the
  derivation tree with its interop obligation, the core API shape) come first.
