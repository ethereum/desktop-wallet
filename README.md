# Ethereum Desktop Wallet (EDW)

> [!WARNING]
> This project is not ready for production-use; the code is unaudited and still under active development.
> Proceed with caution.

## Development

Kohaku dependencies are pinned to a published Git revision in `crates/Cargo.toml` and `crates/Cargo.lock`, including the extended pool catalog. Builds do not require a sibling Kohaku checkout.

To setup dev environment:

```sh
nix develop
```

To run commands

```sh
just run help
# or you can
just build
alias edw=./crates/target/release/edw
edw help
```
