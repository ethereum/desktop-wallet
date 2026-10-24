# Ethereum Desktop Wallet (EDW)

> [!WARNING]
> This project is not ready for production-use; the code is unaudited and still under active development.
> Proceed with caution.

## Development

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

## Implementations & Tooling

- **[kohaku-py](https://github.com/rorupuntou/kohaku-py)**: Pure Python implementation of the EDW engine, Kohaku privacy protocols (Tornado Cash, 0xBow Privacy Pools, Saga Sync), and terminal UI (`edw-py`).

