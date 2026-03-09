{
  description = "Madori (間取り) — GPU application framework for pleme-io apps";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    crate2nix.url = "github:nix-community/crate2nix";
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crate2nix, substrate, ... }: let
    system = "aarch64-darwin";
    pkgs = import nixpkgs { inherit system; };

    rustLibrary = import "${substrate}/lib/rust-library.nix" {
      inherit system nixpkgs;
      nixLib = substrate;
      inherit crate2nix;
    };

    lib = rustLibrary {
      name = "madori";
      src = ./.;
    };
  in {
    inherit (lib) packages devShells apps;

    overlays.default = final: prev: {
      madori = self.packages.${final.system}.default;
    };

    formatter.${system} = pkgs.nixfmt-tree;
  };
}
