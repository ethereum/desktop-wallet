default:
    just --list

build:
    cd contracts && forge soldeer install
    cd contracts && forge build
    cd crates && cargo build --release

test: build
    cd contracts && forge test
    cd crates && cargo test

integration: test
    cd crates && cargo test --test integration

fmt:
    cd crates && cargo fmt && cargo clippy --all-targets --all-features --fix --allow-dirty

ci:
    nix develop .#ci --command bash -c 'cd contracts && forge soldeer install && forge build && forge test'
    nix develop .#ci --command bash -c 'cd crates && cargo check --locked && cargo audit && cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test -- --ignored'

clean:
    cd contracts && forge clean
    cd crates && cargo clean

run *ARGS:
    cargo run --manifest-path crates/bin/Cargo.toml -- {{ARGS}}
