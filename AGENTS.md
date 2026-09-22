# AGENTS.md

What a change has to get right before `just ci` passes, plus the conventions CI cannot check.
Environment setup lives in the [README](README.md) and is not repeated here.

## Layout

- `crates/` is the Rust workspace: `core` (library), `cli` (command implementations), `bin`
  (the `edw` binary). Edition 2024.
- `contracts/` is the Foundry project, dependencies vendored with soldeer.
- `spec/` holds the design documents and is the source of truth for standards.

## Commands

- `just ci` runs what `.github/workflows/ci.yaml` runs, inside `nix develop .#ci`. It is the
  one command worth running before pushing.
- Build the contracts before touching the Rust tests. Four integration test targets bind the
  ABI at compile time through
  `sol!(.., "../../contracts/out/SimpleDelegate.sol/SimpleDelegate.json")`, so without
  `forge build` they do not compile.
- `just fmt` formats and applies clippy's machine-applicable fixes. `just run -- <args>`
  runs the binary.

## Lints deny, they do not warn

- `crates/Cargo.toml` denies clippy's `pedantic` group wholesale, plus `panic`,
  `unwrap_used`, `expect_used`, `unreachable` and `unimplemented`.
- `todo` is only a warning, but CI passes `-D warnings`, so it fails the build too.
- `crates/clippy.toml` relaxes panic, unwrap and expect **in tests only**. A `.unwrap()`
  anywhere else is a build failure: return a `Result` and propagate with `?`. Error enums use
  `thiserror`.
- Two pedantic lints are allowed back: `missing_errors_doc` and `cast_possible_truncation`.

## Item ordering

`arbitrary_source_item_ordering = "deny"` checks module-level order against the groups in
`crates/clippy.toml`:

```
use, modules, macros, global_asm, consts and statics, type aliases, traits,
structs and enums, impls, fns
```

- An `impl` above the `struct` it implements fails, as does a free function above any type
  declared later in the file.
- Order within a group is free, and fields, variants and trait items are not checked.
- Nothing about a file's appearance hints at this, so it is the most common mechanical
  failure here.

## Formatting needs nightly rustfmt

- `crates/.rustfmt.toml` sets `imports_granularity` and `group_imports`, both nightly-only.
- Inside the devshell run `cargo fmt`: nightly rustfmt is ahead of stable on `PATH`, and
  `cargo +nightly fmt` fails there because the shell has no rustup.
- Outside it, stable rustfmt ignores both settings and exits zero, so formatting passes
  locally and fails `cargo fmt --check` in CI.

## Tests

- Unit tests sit beside the code they exercise and may inspect internals.
- Integration tests live in `crates/core/tests/<area>/main.rs`, cover public behavior only,
  keep one property per test, and group helpers at the top of the file.
- Tests that spawn anvil are `#[ignore]`d and run with `cargo test -- --ignored`. CI runs
  both passes. They need a foundry new enough to mine EIP-7702 transactions, which is one
  reason to work inside the devshell.

## Secrets

Not lint-checked, and the reason a change gets rejected in review. See
[`spec/00-vision.md`](spec/00-vision.md) principles 1, 2, 5 and 11.

- Secret material lives in `Zeroizing` or zeroes on drop, and never gets a `Debug`, `Clone`
  or `Serialize` impl that copies it out. `crates/cli/src/unlock.rs` is the pattern for
  passwords.
- Nothing secret reaches logs, tracing fields or error messages.
- Outbound network calls are RPC only. Telemetry, analytics, crash reporting, indexers,
  price feeds and update checks are principle violations, not features.
- A change touching keys, signing, storage, derivation or mixing needs a second reviewer
  signing off on secret handling specifically.

## Comments and docs

Standards in
[`spec/01-architecture.md`](spec/01-architecture.md#cross-cutting-engineering-standards).

- Public surface follows the
  [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).
- A comment earns its place by carrying intent the code cannot. Never narrate the next line.
- Doc comments open with a one-sentence summary, then add only what a caller cannot infer
  from the signature and the types.
- Trait definitions are where that budget is worth spending, since the doc is the interface.
  Implementation files stay light.
- `# Errors` is convention rather than enforcement, since `missing_errors_doc` is allowed.

## Dependencies

- CI runs `cargo check --locked` and `cargo audit`. Commit `crates/Cargo.lock` with any
  dependency change, and expect a new advisory to break an otherwise untouched branch.
- Shared versions belong in `[workspace.dependencies]`, with members taking
  `{ workspace = true }`.

## Contracts

- `forge build` and `forge test` both run in CI. Solidity tests live in `contracts/test/`.
- Dependencies are installed with `forge soldeer install` and pinned in
  `contracts/soldeer.lock`.
- `contracts/foundry.toml` disables the metadata hash so bytecode stays deterministic.

## Commits and branches

- Branch off `master`, named `type/topic`: `docs/agents-md`, `feat/keystore-key-slots`.
- Conventional-commit subjects (`feat`, `fix`, `docs`, `test`, `ci`, `refactor`, `style`,
  `build`), wrapped at 72 columns, with a body saying why rather than what.
