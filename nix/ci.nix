{
  pkgs,
  rust,
  rustfmt,
  ...
}:
pkgs.mkShell {
  packages = with pkgs; [
    rustfmt
    rust
    cargo-audit

    foundry
  ];
}
