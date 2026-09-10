self: {
  pkgs,
  rust,
  rustfmt,
  ...
}:
pkgs.mkShell {
  packages = with pkgs; [
    rust
    rustfmt
    rust-analyzer
    bacon
    cargo-audit
    cargo-autoinherit
    cargo-sort

    foundry

    just
    nodejs_24
    pnpm_11
  ];

  shellHook = ''
    just
    alias edw='./crates/bin/target/release/edw'
  '';
}
