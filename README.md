# Ethereum Desktop Wallet (EDW)

> [!WARNING]
> This project is not ready for production-use; the code is unaudited and still under active development.
> Proceed with caution.

## Development

EDW primarily uses [nix flakes](https://wiki.nixos.org/wiki/Flakes) for development. See the [flake.nix](./flake.nix) file for details on required dependencies. To enter the dev shell, run:

```bash
nix develop --extra-experimental-features "nix-command flakes"
```

## Common commands

EDW provides a [justfile](./justfile) for common commands. For information on the CLI, see [docs/cli](./docs/cli). To see a list of available development commands, run:

```bash
just --list
```

## Implementations & Tooling

- **[kohaku-py](https://github.com/rorupuntou/kohaku-py)**: Pure Python implementation of the EDW engine, Kohaku privacy protocols (Tornado Cash, 0xBow Privacy Pools, Saga Sync), and terminal UI (`edw-py`).

