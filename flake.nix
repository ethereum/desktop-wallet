{
  description = "edw flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [rust-overlay.overlays.default];
      };

      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = [
          "rust-src"
          "llvm-tools"
        ];
        targets = ["wasm32-unknown-unknown"];
      };

      rustfmt = pkgs.rust-bin.nightly.latest.rustfmt;
    in {
      packages.default = import ./nix/package.nix {inherit pkgs rust;};

      devShells = {
        default = import ./nix/devshell.nix {inherit pkgs rust rustfmt;} self;
        ci = import ./nix/ci.nix {inherit pkgs rust rustfmt;} self;
      };
    })
    // {
      nixosModules.default = import ./nix/module.nix self;
    };
}
