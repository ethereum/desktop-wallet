# Command-line surface

> **Status: proposed.** A concrete proposal for review, not an agreed surface. Where a choice
> follows from a principle in [`00-vision.md`](./00-vision.md) it is made here and the principle
> is cited, so a reviewer can check the reading. One question is genuinely open and is called
> out at the end. The conventions are the part worth agreeing first; the command list follows
> from them and will keep moving.

The CLI ships before the GUI, it is the surface an automated harness drives, and its output is
what a test asserts against. That makes it an interface contract in the same sense as the
`edw-core` API in [`01-architecture.md`](./01-architecture.md), and it is held to the same bar.

Everything here is scoped to the `v0.1.0` release.

## Prior art

[`kohaku-cli`](https://0000000000.org/commands) covers this problem domain and is the reference
for the coverage `edw` needs. The shape of the surface comes from the wallet and profile model
below.

## The model this sits on

**Each network is its own wallet instance.** One directory under the data dir, one decryption
password, one set of profiles and settings. `edw unlock --network <name>` opens an instance,
later commands inherit it from the session, and only one is unlocked at a time. Inside an
instance, `networkConfig` rows hold the RPC endpoints and are all pinned to that instance's
chain ID, so there is no wallet-wide network list to enumerate.

**Profiles are what users name.** Mnemonics are numbered background state (`mnemonic 0`,
`mnemonic 1`) with no commands of their own. First unlock of a new instance generates a
12-word seed, creates mnemonic 0 and profile 0, and prints the phrase once. Display names are
unique within an instance, and an unnamed index 0 displays as `default`.

Both are defined in [`vocabulary.md`](./vocabulary.md). Signers, executors, and vaults stay
core objects; they are not command nouns.

## Grammar

**Proposed: two families, not one `edw <noun> <verb>` surface.**

Prefixing every action with `vault` or `signer` makes the user think in core types, which
fights [`00-vision.md`](./00-vision.md) principle 9 (think in Profiles, not addresses). Noun-verb
is useful for *setup*; day-to-day work is a verb.

- **Config** is `edw <noun> <verb>` for adding, changing, listing, and inspecting setup.

| Noun      | Holds                                                         |
| --------- | ------------------------------------------------------------- |
| `profile` | the user-facing aggregation of signers, executors, and vaults |
| `network` | the current instance's networkConfigs and their endpoints     |
| `db`      | the encrypted store itself                                    |
| `config`  | resolved configuration, for inspection                        |

- **Actions** are top-level verbs (`edw transfer`, `edw shield`, `edw balance`, …). When an
  action needs a profile, it takes `--profile <name>`. Omitted `--profile` is not silently
  the display name `default` when other profiles exist; see [Missing arguments](#missing-arguments).
- **Session** verbs (`unlock`, `lock`) stay top-level. They belong to no config noun.

[`00-vision.md`](./00-vision.md) principle 8 puts the safe path on the easy path, so the
primary value-moving flow is the short spelling `edw transfer` (stealth by default). There is
no `edw asset transfer` alias, and `signer` / `vault` / `asset` are not command nouns.

Names are kebab-case throughout, including flags.

## The session replaces per-command credentials

**Proposed: no command outside `unlock` accepts a password or names an instance.** The
instance is chosen by `edw unlock --network <name>`, the password is held for the session,
and every later command inherits both. For automation the password comes from
`EDW_DECRYPTION_PASSWORD`, which keeps it out of `argv`. A locked wallet fails with an
instruction, never with a prompt a script cannot answer.

This is the one place the prior art should not be followed. `kohaku-cli` takes `--password` on
nearly every command and accepts a literal, which any local process can read out of `ps` and
which lands in shell history; its file-path form exists to work around that.

### Global flags

| Flag                | Source              | Meaning                                         |
| ------------------- | ------------------- | ----------------------------------------------- |
| `--data-dir`        | `DATA_DIR`          | root of the wallet instances                    |
| `--rpc-url`         | `RPC_URL`           | overrides the active endpoint for one command   |
| `--non-interactive` | flag only           | never prompt; emit JSON; fail if input is short |
| `--broadcast`       | flag only, mutating | actually submit; otherwise dry-run              |

`--network` is deliberately **not** global. The network is a property of the unlocked session
rather than of each command, so it is an argument to `edw unlock` alone. `--profile` is
likewise **not** global: it belongs on actions, and is filled in when omitted as described
in [Missing arguments](#missing-arguments).

## What happens on an empty or locked instance

This is the one place the two halves of the model pull against each other.

Opening an instance never happens as a side effect: a command against a locked wallet fails
and says to run `edw unlock`. But first unlock of a _new_ instance does create state, since it
generates a seed and profile 0 so the user has something to hold.

**Proposed: `unlock` is the single command allowed to create, and it creates only on first
use of an instance.** Everything else fails rather than bootstrapping. Stated as a rule with
one named exception, so neither behavior is a surprise:

- `edw <anything>` on a locked instance: fails, tells the user to unlock.
- `edw unlock --network <name>` on a fresh instance: creates it, generates mnemonic 0 and
  profile 0, prints the phrase once.
- `edw unlock --network <name>` on an existing instance: opens it, creates nothing.

The alternative is an explicit `edw init` that first unlock refuses to stand in for. That is
more predictable and one more step in every quickstart.

## Output

Two modes, and the distinction is a contract because a harness depends on it.

**Default is for a person.** Tables, aligned columns, whatever reads best. Nothing may depend
on this format.

**`--non-interactive` is for a machine.** One JSON document on stdout, no prompts, and a
non-zero exit when a required input is missing rather than a prompt for it. Diagnostics go to
stderr so stdout stays parseable.

Whether that JSON shape carries the same compatibility promise as the `edw-core` API is the
one question this document leaves open, below.

### Every prompt has a non-interactive equivalent

`--non-interactive` is a property of the whole surface rather than a feature some commands
have. Two obligations follow, and they constrain how each command is designed:

- **Every input a command can prompt for also has a flag or an environment variable.** A
  command that can only obtain an input by asking is unfinished. `EDW_DECRYPTION_PASSWORD` is
  that equivalent for the password.
- **Confirmations count as input.** For anything that submits a transaction, `--broadcast` is
  the confirmation, so dry-run by default already satisfies this.

### Missing arguments

A missing argument that has a finite, known set of choices is filled in. A missing argument
that does not is still an error, or a labeled typed prompt if that is the only way to get it.
`--profile` is the example the rest of the surface copies; config commands that already take
an optional chooser (`network set-rpc --name`) follow the same rule.

Do **not** silently bind omitted `--profile` to the display name `default` when other profiles
exist. That is a footgun on `transfer` and `shield`.

- **Interactive (TTY, not `--non-interactive`):**
  - Exactly one legal value (one profile, one networkConfig, …): use it, no prompt.
  - Several legal values: print a numbered list and read a choice.
  - Open-ended values (`--to`, amounts, a phrase): prompt with a label, or fail if a prompt
    would be unsafe (seed material already has its own TTY rules).
- **`--non-interactive`:** never prompt. Omitted `--profile` is fine only when there is
  exactly one profile; otherwise fail and name the flag. Same for any other chooser.

One command is exempt from `--non-interactive` entirely, for the reason in the next section.

### Secrets never reach the machine path

Principle 11 in [`00-vision.md`](./00-vision.md) is "no secrets in logs. Ever." A CLI's stdout
is terminal scrollback, a shell history, and sometimes a CI artifact, so that principle applies
directly here:

- **No secret material in `--non-interactive` output, from any command.** That is the path
  that gets redirected to a file and archived by a runner.
- **Interactive reveal requires a TTY and an explicit confirmation.** `edw profile reveal-seed`
  refuses to run when stdout is not a terminal.
- **`edw profile reveal-seed` has no `--non-interactive` mode**, and is the one exception to
  the rule above. A machine-readable seed phrase is the thing that rule exists to prevent.

The command stays. The wallet already prints a seed phrase once at first unlock, so seed
material on a terminal is a flow the product has; a backup command the user can re-run
deliberately is safer than leaving them to screenshot the one-time print. Note that
[`00-vision.md`](./00-vision.md) scopes the safety basic to a "seed-backup flow at creation",
so a re-runnable command extends it and is worth confirming.

## Mutating commands dry-run by default

A command that moves value prints what it would do and exits; `--broadcast` submits. The
dry-run is expected to do real work (simulate, estimate fees, resolve recipients) so that the
difference between the two runs is only the submission. Taken from the prior art unchanged.

**`--broadcast` guards chain state, not every durable effect.** `edw profile generate` writes
to disk without it. A profile is recoverable from the seed phrase the wallet prints at first
unlock; a broadcast transaction is recoverable from nothing. Dry-run guards the irreversible
half.

## Selectors and amounts

**Polymorphic selectors.** `--from` accepts an address, an HD index, or a stealth selector;
`--to` accepts an address, a name, or a stealth address. This is right for a wallet whose
premise is that the user should not be thinking about addresses. Resolution order has to be
unambiguous and specified per flag.

**Three amount flags.** `--amount-wei` for exact base units, `--amount-formatted` for a decimal
amount converted using the asset's decimals, `--amount-max` for the full balance. Exactly one
is required. Overloading a single `--amount` with a unit suffix puts a parser between the user
and the amount of money they are moving.

## Commands

All of these are in `v0.1.0`, against the capability list in [`00-vision.md`](./00-vision.md).

### Session

| Command      | Does                                                    |
| ------------ | ------------------------------------------------------- |
| `edw unlock` | opens an instance for the terminal session; `--network` |
| `edw lock`   | ends the session                                        |

### Config

| Command                     | Does                                                |
| --------------------------- | --------------------------------------------------- |
| `edw profile generate`      | new mnemonic plus one profile at `--index`          |
| `edw profile import`        | existing phrase; re-importing a stored one is error |
| `edw profile add`           | another profile on a stored mnemonic                |
| `edw profile list`          | profiles in the unlocked instance                   |
| `edw profile rename`        | display names are unique within an instance         |
| `edw profile reveal-seed`   | re-runnable seed backup; TTY and confirmation gated |
| `edw network list` / `add`  | networkConfigs within the instance                  |
| `edw network set-rpc`       | endpoint for a networkConfig                        |
| `edw network endpoint list` | endpoints on a networkConfig                        |
| `edw network status`        | chain ID and block height                           |
| `edw network traffic`       | what the CLI contacted                              |
| `edw db path` / `migrate`   | store location and migrations                       |
| `edw db purge`              | deletes profile database data                       |
| `edw config view` / `path`  | resolved configuration                              |

`kohaku-cli`'s `create-wallet` has no single counterpart, because "wallet" means the
per-network instance here while the user-named thing is a profile. The three creating
commands replace it.

**`edw network traffic` is worth taking early.** It makes principle 2 ("limit egress")
observable, and it is the same enumeration an egress-allowlist test asserts against.

### Actions

Actions take `--profile` when they need a profile. Fill-in follows [Missing arguments](#missing-arguments).

| Command               | Does                                                     |
| --------------------- | -------------------------------------------------------- |
| `edw transfer`        | stealth is the default path, not a flag                  |
| `edw shield`          | deposit to the shielded pool                             |
| `edw unshield`        | withdraw, including the private-gas path                 |
| `edw balance`         | the aggregated per-profile view                          |
| `edw stealth-address` | show the stealth address others send to                  |
| `edw next-address`    | the invisible fresh address                              |
| `edw history`         | transaction history                                      |
| `edw call`            | contract-call send from a profile account                |

**`--protocol` is kept on `shield` / `unshield`, defaulting to `tornado`.** `edw` supports one
shielded pool, so the flag has a single legal value today and could be argued away. Keeping it
costs a default and buys two things. The surface does not change shape when a second pool
arrives, so no script written against `v0.1.0` breaks. And the pool is named in the command
that moves value into it, which is what principle 6 asks for: the anonymity set a user is
joining is the privacy property they are buying, and it should be legible rather than implied.

An unrecognized value is an error listing what is supported, so the flag never silently
resolves to something other than what was asked for.

## Open question

**Whether the `--non-interactive` JSON shape carries the same compatibility promise as the
`edw-core` API from v0.1.0, or is explicitly unstable until v1.** Nothing in
[`00-vision.md`](./00-vision.md) settles this. It is asymmetric: a promise can be added later
but not withdrawn, and adding it late breaks every harness built in the meantime at least
once.

**Proposed: explicitly unstable until v1**, on the reversibility argument, with the shape
documented from the start so that promising it later is a formality rather than a redesign.
