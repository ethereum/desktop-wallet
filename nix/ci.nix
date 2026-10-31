{
  pkgs,
  rust,
  rustfmt,
  alto,
  ...
}:
pkgs.mkShell {
  packages = with pkgs; [
    rustfmt
    rust
    cargo-audit

    foundry
    alto
  ];
}
