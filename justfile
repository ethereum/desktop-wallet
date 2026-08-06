default:
    just --list

build:
    cd crates/bin && cargo build --release

run:
    cd crates/bin && cargo run
