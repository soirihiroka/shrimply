{
  description = "Shrimply development environment and package";

  nixConfig = {
    extra-substituters = [ "https://cache.nixos-cuda.org" ];
    extra-trusted-public-keys = [
      "cache.nixos-cuda.org:74DUi4Ye579gUqzH4ziL9IyiJBlDpMRn9MBN8oNan9M="
    ];
  };

  inputs = {
    manim = {
      url = "github:3b1b/manim/9d57bcf9edea2486f214e190931de2a5537f23c1";
      flake = false;
    };
    nixpkgs.url = "https://github.com/NixOS/nixpkgs/archive/dc5d91f840324650bac8c379428c7037a416959a.tar.gz";
    optix = {
      url = "github:NVIDIA/optix-dev/f1f6dd803f3159992d248178f6e09421c6eb8b6d";
      flake = false;
    };
    rhubarb = {
      url = "github:DanielSWolf/rhubarb-lip-sync/9b9573cd21b253c9ba58739bbd1aa0b50b991bff";
      flake = false;
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay/ca7f624be3935a5bc46d2c240515491ab8675503";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    vtracer = {
      url = "github:visioncortex/vtracer/1ddc9ebbf7120af7d2b92518f1b56ddd95430db1";
      flake = false;
    };
  };

  outputs =
    {
      self,
      manim,
      nixpkgs,
      optix,
      rhubarb,
      rust-overlay,
      vtracer,
      ...
    }:
    let
      system = "x86_64-linux";
      overlay =
        final: prev:
        let
          rustToolchain =
            (import rust-overlay final prev).rust-bin.fromRustupToolchainFile
              ./rust-toolchain.toml;
          rustPlatform = final.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
        in
        {
          shrimply-slang = prev.callPackage ./pkgs/slang.nix { };
          shrimply =
            (final.callPackage ./pkgs/shrimply.nix {
              inherit rustPlatform;
              manimSrc = manim;
              optixSrc = optix;
              rhubarbSrc = rhubarb;
              vtracerSrc = vtracer;
            }).overrideAttrs
              {
                src = self;
              };
        };
      pkgs = import nixpkgs {
        inherit system;
        overlays = [
          (import rust-overlay)
          overlay
        ];
        config.allowUnfree = true;
      };
      rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
    in
    {
      overlays.default = overlay;
      packages.${system} = {
        shrimply = pkgs.shrimply;
        default = pkgs.shrimply;
      };
      devShells.${system}.default = pkgs.callPackage ./pkgs/dev-shell.nix {
        inherit rustToolchain;
      };
    };
}
