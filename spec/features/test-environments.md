# Feature: Test environments

**Status:** draft
**Owner:** unassigned
**Milestone:** open, see [Open questions](#open-questions)
**Related:** [EDW-005 (#30)](https://github.com/ethereum/desktop-wallet/issues/30),
[EDW-006 (#32)](https://github.com/ethereum/desktop-wallet/issues/32),
[EDW-013 (#39)](https://github.com/ethereum/desktop-wallet/issues/39),
[EDW-014 (#40)](https://github.com/ethereum/desktop-wallet/issues/40),
[EDW-019 (#46)](https://github.com/ethereum/desktop-wallet/issues/46),
[EDW-021 (#31)](https://github.com/ethereum/desktop-wallet/issues/31),
[`01-architecture.md`](../01-architecture.md)

## Problem

Three issues say a flow must work "end to end on a testnet" (EDW-013, EDW-014, EDW-019), and
nothing says which testnet, at which block, with which hardfork. There is no environment
behind those words yet.

The concrete version of the gap is already in the repo. The anvil-backed tests pass in CI,
where nix pins foundry, and fail on a contributor machine whose foundry defaults to a
pre-Prague hardfork, because EIP-7702 authorization is rejected. That is a small bug with a
general shape: the chain the tests run against is implicit, so it differs per machine and no
result is reproducible across them.

The second, larger motivation is to make the codebase safe to probe adversarially, including
by AI agents running unattended. That is attractive because the properties this wallet claims
are mostly machine-checkable: no plaintext secrets at rest (principle 5), only-RPC egress
(principle 2), no secret material in logs or send paths (principle 11), and the trust
boundary in [`01-architecture.md`](../01-architecture.md). An agent that can run a flow,
observe disk, observe egress, and check those properties is doing real work. An agent without
those checks produces plausible prose about vulnerabilities that do not exist, and triaging
that costs more than it returns.

So the proposal treats the environment and the checks as one deliverable. The environment
makes a finding reproducible; the checks make a finding true.

## Behavior

### Two tiers

Revised down from three during review: a separate bare devnet and pinned fork were largely
the same environment, so they are merged.

| Tier       | Runs against                                                           | Purpose                                                                   | Reproducible |
| ---------- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------ |
| **devnet** | local anvil, booted from a materialized mainnet fork at a pinned block | the invariant suite, property tests, agent probing, everyday `cargo test` | yes, fully   |
| **live**   | Sepolia, nightly                                                       | real providers and services, rate limits, reorgs, mock drift              | no           |

Both are selected the same way a user selects a network, through the runtime endpoint and
chain-ID configuration EDW-005 already requires, so the environments exercise the shipping
code path rather than a test-only one.

**Why a fork rather than a bare chain.** The shielded pools EDW-013 targets are mainnet
deployments; their testnet equivalents have no meaningful anonymity set, and in some cases no
deployment at all. Forking mainnet gets the real contracts, real token behavior including the
non-standard ERC-20s, and real historical state for the chunked `eth_getLogs` sync.

**Why it must be hermetic, and how.** A fork that reaches a live archive endpoint on every
cold state access would put a metered network dependency in the inner loop of `cargo test`,
which is a poor fit for a project whose principle 2 is about minimizing egress. It also
weakens the determinism claim, since such a fork is reproducible only insofar as a provider
keeps returning the same data, and providers differ on pruning and trace availability.

The proposal is therefore to **materialize** the fork rather than live-fork on every run:
fork at the pinned block once, exercise the state the tests need, and dump it with anvil's
`--dump-state`. Runs then boot from that dump with `--load-state` and no network at all. The
result has real mainnet contracts, is fast, and is reproducible from a committed or cached
file rather than from an external service. Whether the dump stays a workable size, and
replays faithfully, is the main unknown; see [Open questions](#open-questions).

**Why the live tier survives the merge.** Its purpose is narrower and sharper than "real
reorgs": it is what catches **mock drift**. Every fake in the devnet tier is at risk of being
kinder than the service it stands in for, with no rate limits, no pagination quirks, and no
reorg handling, so a suite that passes against fakes can still meet a production failure. A
periodic run against the real thing is the only check on that. It stays a nightly soak rather
than the probing surface, because nothing found there replays on demand.

### Third-party services

Raised in review, and the harder half of the fork idea: services outside the chain, such as
an indexer, a bundler, a relayer, or an association-data provider, are correct only up to the
pinned block and diverge after it.

Three things are proposed, in order:

1. **The mock surface is EDW-021's enumeration.** That issue already requires every outbound
   host to be listed in one place, with what it sees about the user. That list is exactly the
   set of things needing fakes here, so the two pieces of work are one piece. Nothing can be
   mocked that has not been enumerated.
2. **Ask whether the service survives before faking it.** Principle 2 holds that anything
   which could come from RPC should, and names indexers specifically. Building a faithful
   fake for a dependency that ought to be removed is wasted effort, and worse, it entrenches
   the dependency by making it cheap to keep.
3. **For whatever does survive, record and replay rather than reimplement.** Capture real
   responses at the same block the fork is pinned to, and serve them back. Reimplementing a
   service's behavior is a large job that drifts from the original. For the period after the
   pin, the local chain is the source of truth, so post-pin data is derivable from what anvil
   produced rather than invented.

### The invariant suite

The centerpiece. A catalog of properties, each an executable test that fails loudly, which an
agent tries to break and can extend. The existing `no_plaintext_key_material_on_disk` test
from EDW-003 is the intended shape: it writes a profile through the real repository traits,
then scans every byte the store produced for key material in several encodings.

Proposed starting set:

- No plaintext secret material at rest, in any backend, after any flow.
- No secret material in logs, stdout, or error text, including on failure paths.
- No outbound connection to a host outside the declared allowlist. EDW-021 already requires
  this test; here it also runs continuously and per flow.
- No panic reachable from any send or signing path.
- Value conservation across a round trip: shield then unshield, or deposit then withdraw,
  leaves no unaccounted balance.
- Derivation matches its committed known-answer vectors.
- A profile written by one build is readable by the next, or fails with a versioned error
  rather than corrupt state.

### The agent loop

Proposed shape, deliberately narrow at the start: an agent gets the devnet tier, the CLI's
`--non-interactive` JSON surface from EDW-006, and the invariant suite. It runs flows, tries
to break an invariant, and when one breaks it opens a PR containing the failing test.

The governing rule: **a finding is a failing test, or it is not a finding.** An agent PR that
does not include a test that reproduces the issue is closed. This is what keeps the loop from
becoming review load, and it costs nothing when the finding is real.

## Non-goals

- **Not a deployed staging service.** A wallet has no production deployment to clone; it runs
  on the user's machine. "Production-like" here means the chain and the host conditions, not
  a hosted environment.
- **Not a replacement for CI.** CI stays the gate on every PR. These tiers are where longer,
  slower, and adversarial work runs.
- **No mainnet testing with real value.** No tier holds funds of consequence.
- **No auto-merge of agent PRs.** They enter the same review path as any other, including the
  second-reviewer gate on anything touching keys, signing, storage, or the trust boundary.
- **Not a public conformance suite yet.** The invariant suite may become one, see
  [Open questions](#open-questions), but that is a separate decision with a wider contract.

## Core / UI contract

This adds no `wallet-core` API. It depends on two surfaces that other issues already define,
and proposes treating them as stable contracts because agents and harnesses will build
against them:

- **Network selection (EDW-005).** Endpoint and chain ID configurable at runtime, with the
  profile bound to a chain ID so a mismatched RPC is rejected. Each tier is a configuration,
  not a build.
- **The machine surface (EDW-006).** `--non-interactive` JSON output on every command that
  produces data, and dry-run by default with an explicit `--broadcast`. This is what an agent
  drives. Proposed: treat its schema as a documented contract, so a harness does not break on
  incidental output changes.

What must never cross into an agent environment: mainnet RPC credentials, any key holding
value, and any credential that is not scoped to the tier it runs in.

## Acceptance criteria

- [ ] The devnet tier is a fixture: pinned hardfork, pinned fork block, fixed mnemonic,
      deterministic chain state, with snapshot and revert helpers. Two runs from the same seed
      produce the same result.
- [ ] The devnet tier runs with no network access, so a run cannot depend on a live archive
      endpoint. The fork block is recorded in the repo rather than passed by hand.
- [ ] The anvil-backed tests run against that fixture and no longer depend on the ambient
      foundry default. The pre-Prague failure does not recur.
- [ ] Every third-party service the app can reach is either eliminated per principle 2 or has
      a recorded fixture, and the set matches EDW-021's enumeration rather than being tracked
      separately.
- [ ] The invariant suite exists as a named, runnable target, with at least the starting set
      above, and runs in CI on every PR.
- [ ] The live tier runs on a schedule, not on every PR, and its failures are triaged
      separately from CI failures so a flake does not read as a regression.
- [ ] An agent environment has no credential or key that reaches beyond its tier, enforced
      rather than documented.
- [ ] Egress from any tier is confined to a declared allowlist, and a connection outside it
      fails the run. This is the EDW-021 test wired to the environment.
- [ ] The contribution rules state that an agent-authored PR must include a reproducing test,
      and that agent PRs are subject to the existing security-review gate.

## Security considerations

**The harness is itself an attack surface.** It runs untrusted-ish input against code that
handles keys, on machines with network access and repository credentials. The mitigations
proposed are the same ones the wallet claims for itself: no credential broader than the tier,
network denied by default with an explicit allowlist, and no funded key anywhere an agent can
reach. The allowlist has a useful second effect: it makes principle 2 continuously tested
rather than tested once.

**Agent-authored PRs are a supply-chain path.** An agent that can open PRs against a wallet
repository is a route into shipped code. The reproducing-test rule is a quality filter, not a
security control; the security control is that agent PRs get no special trust, do not
auto-merge, and hit the second-reviewer requirement like anything else touching secrets.

**A finding is sensitive before it is fixed.** If the loop works, it will eventually produce a
real vulnerability in a public repository. Worth deciding in advance where those go, since
"open a PR with a failing test" is the wrong default for a live key-handling bug.

**What an auditor should look at:** whether the invariant suite's properties actually match
the claims in `00-vision.md`, and whether any tier can reach a real key or a real endpoint.

## Test plan

The environments need testing themselves, which mostly means proving determinism:

- Same devnet seed twice produces byte-identical results, so a reported failure replays.
- The materialized fork produces the same protocol state across machines, booted from the
  dump rather than from a provider.
- Each invariant has a deliberately broken build that it catches, so the suite is known to
  fail when it should rather than passing vacuously. This is the most important test in the
  plan: an invariant that cannot fail is worse than no invariant, because it reads as a
  guarantee.
- The egress allowlist is verified by an attempted connection to an undeclared host.

Nothing here is user-visible, so no GUI verification applies.

## Open questions

1. **Milestone.** The devnet tier arguably belongs in v0.1.0, since EDW-013, EDW-014 and
   EDW-019 all depend on "end to end on a testnet" and the pre-Prague failure is present
   today. The live tier and the recorded service fixtures read as v0.2.0. Proposal: split on
   that line rather than scheduling the whole thing at once.
2. **Who runs the agents, and where.** Local, CI-hosted, or a dedicated machine. This decides
   how much of the containment above is enforced by infrastructure versus convention.
3. **Whether the materialized fork is practical.** Producing the dump needs archive access
   once, which is a smaller ask than needing it per run, but the dump has to stay a workable
   size and replay faithfully. Proposed as a timeboxed spike before the shape is committed to,
   with a live fork as the fallback if it does not hold up.
4. **Which third-party services survive EDW-021.** This decides what needs a recorded fixture
   and what needs deleting instead, and it gates the work in
   [Third-party services](#third-party-services).
5. **Where vulnerability findings go** before they are fixed, given this is a public
   repository.
6. **Whether the invariant suite becomes a public conformance artifact.** As a reference
   implementation, a suite other wallets can run against their own builds fits the mission,
   and would make this spec-level rather than internal tooling. It also raises the bar on its
   API and its stability, so it is worth deciding deliberately rather than by drift.
