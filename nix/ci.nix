{
  pkgs,
  rust,
  rustfmt,
  alto,
  ...
}:
pkgs.mkShell {
  nativeBuildInputs = [pkgs.rustPlatform.bindgenHook];
  packages = with pkgs; [
    rustfmt
    rust
    cargo-audit

    foundry
    alto
  ];
}
