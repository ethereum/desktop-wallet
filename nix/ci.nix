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
    cargo-audit

    foundry
  ];
}
