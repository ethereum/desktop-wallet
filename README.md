# Ethereum Desktop Wallet (EDW)

> [!WARNING]
> This project is not ready for production-use; the code is unaudited and still under active development.
> Proceed with caution.

## Development

The wallet currently uses the sibling `../kohaku-rs` checkout for its Kohaku dependencies, including the Extended pool catalog. Keep these repositories beside one another while developing this coordinated change. A standalone release should replace those path dependencies with a Git revision after the Kohaku changes have been published.

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
