{
  pkgs,
  rust,
  ...
}: let
  lib = pkgs.lib;

  rustPlatform = pkgs.makeRustPlatform {
    cargo = rust;
    rustc = rust;
  };
in
  rustPlatform.buildRustPackage {
    pname = "edw";
    version = (lib.importTOML ../crates/bin/Cargo.toml).package.version;

    src = lib.fileset.toSource {
      root = ../crates;
      fileset = lib.fileset.difference ../crates (lib.fileset.maybeMissing ../crates/target);
    };

    cargoLock.lockFile = ../crates/Cargo.lock;

    nativeBuildInputs = with pkgs; [
      cmake
      perl
      pkg-config
    ];

    meta = {
      description = "Ethereum desktop wallet";
      mainProgram = "edw";
      platforms = lib.platforms.unix;
    };
  }
