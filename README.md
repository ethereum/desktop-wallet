# desktop-wallet

ethereum desktop wallet

## Development

The easiest way to get a stable dev environment is nix. [Install Nix here](https://nixos.org/download/), then run:

```shell
nix develop --extra-experimental-features "nix-command flakes" --command $SHELL
```

Work inside that shell. It pins the toolchain CI uses, which matters in two places that fail
quietly otherwise:

- **foundry.** The anvil-backed tests need a build that mines EIP-7702 transactions. An older
  foundry spawns and accepts them, then drops them during block building, so a test hangs
  rather than failing. `devnet_with_7702()` probes for this and fails in ten seconds.
- **rustfmt.** `crates/.rustfmt.toml` sets `imports_granularity` and `group_imports`, both
  nightly-only. Stable `cargo fmt` ignores them and exits zero, so formatting can pass
  locally and fail in CI.

```shell
cd crates
cargo test --workspace              # unit and non-devnet tests
cargo test --workspace -- --ignored # the anvil-backed tests
cargo clippy --workspace --all-targets
cargo fmt --all
```
