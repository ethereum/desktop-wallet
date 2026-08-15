default:
    just --list

build:
    cd contracts && forge build
    cd crates && cargo build

test:
    cd contracts && forge test
    cd crates && cargo test

integration: test
    cd crates && cargo test --test integration

clean:
    cd contracts && forge clean
    cd crates && cargo clean

run:
    cd crates/bin && cargo run
