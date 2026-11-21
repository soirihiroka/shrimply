{
  description = "Shrimply development environment and package";

  nixConfig = {
    extra-substituters = [
      "https://cache.nixos-cuda.org"
      "https://shrimply.cachix.org"
    ];
    extra-trusted-public-keys = [
      "cache.nixos-cuda.org:74DUi4Ye579gUqzH4ziL9IyiJBlDpMRn9MBN8oNan9M="
      "shrimply.cachix.org-1:cAgdyBbGPgJZMWUP4PAQPVENeCYtGT7OPPkocvo+uuA="
    ];
  };

  inputs = {
    self.submodules = true;
    nixpkgs.url = "https://github.com/NixOS/nixpkgs/archive/dc5d91f840324650bac8c379428c7037a416959a.tar.gz";
    rhubarb = {
      url = "github:DanielSWolf/rhubarb-lip-sync/9b9573cd21b253c9ba58739bbd1aa0b50b991bff";
      flake = false;
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay/ca7f624be3935a5bc46d2c240515491ab8675503";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rhubarb,
      rust-overlay,
      ...
    }:
    let
      system = "x86_64-linux";
      # The GitHub flake fetcher downloads an archive without any submodules.
      # Fetch that same project revision through Git to resolve its gitlinks.
      src =
        if builtins.pathExists (self + "/external/ffmpeg/RELEASE") then self
        else if self.sourceInfo ? submodules || !(self ? rev) then
          throw "Initialize the project submodules with git submodule update --init --recursive."
        else builtins.fetchGit {
          url = "https://github.com/soirihiroka/shrimply.git";
          rev = self.rev;
          submodules = true;
          shallow = true;
        };
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
          shrimply-ffmpeg =
            let
              source = builtins.path { path = src + "/external/ffmpeg"; name = "ffmpeg"; };
              codec = (prev.ffmpeg.override {
                inherit source;
                withUnfree = true;
                withFdkAac = true;
                version = final.lib.removeSuffix "\n" (builtins.readFile (source + "/RELEASE"));
              }).overrideAttrs (old: {
                # The pinned nixpkgs recipe still configures the removed OpenGL output.
                configureFlags = builtins.filter (flag: flag != "--disable-opengl") old.configureFlags;
                postPatch = (old.postPatch or "") + "\ncp RELEASE FF_VERSION\n";
              });
            in final.symlinkJoin {
              name = "shrimply-ffmpeg-${codec.version}";
              paths = [ (final.lib.getBin codec) (final.lib.getLib codec) (final.lib.getDev codec) ];
            };
          shrimply-slang = prev.callPackage ./pkgs/slang.nix { };
          shrimply-oidn = prev.callPackage ./pkgs/oidn.nix { };
          shrimply = final.callPackage ./pkgs/shrimply.nix {
            inherit rustPlatform src;
            ffmpeg = final.shrimply-ffmpeg;
            rhubarbSrc = rhubarb;
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
        ffmpeg = pkgs.shrimply-ffmpeg;
      };
    };
}
