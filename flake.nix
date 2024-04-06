{
  inputs = {
    naersk.url = "github:nmattia/naersk/master";
    # This must be the stable nixpkgs if you're running the app on a
    # stable NixOS install.  Mixing EGL library versions doesn't work.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-23.11";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-compat = {
      url = github:edolstra/flake-compat;
      flake = true;
    };
  };

  outputs = { self, nixpkgs, utils, naersk, rust-overlay, ... }:
    utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {inherit system overlays;};
        naersk-lib = pkgs.callPackage naersk {
            cargo = pkgs.rust-bin.stable.latest.default;
            rustc = pkgs.rust-bin.stable.latest.default;
        };
        manifest = (builtins.fromTOML (builtins.readFile ./app/Cargo.toml)).package;
        commonBuildInputs = with pkgs; [
          gsettings-desktop-schemas #https://nixos.org/manual/nixpkgs/unstable/#ssec-gnome-common-issues
          xorg.libxcb
          gtk3.dev
          pkg-config

          libGL
          libxkbcommon
          wayland
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          xorg.libxcb
          fontconfig
        ];
        runtimeDependencies = with pkgs; [
          libGL
          libxkbcommon
        ];
      in
      {
        defaultPackage = naersk-lib.buildPackage {
          src = pkgs.lib.cleanSource ./.;
          doCheck = true;
          pname = manifest.name;
          nativeBuildInputs = [
            pkgs.autoPatchelfHook
            pkgs.wrapGAppsHook
          ];
          runtimeDependencies = runtimeDependencies;
          buildInputs = with pkgs; [
            pkgs.rust-bin.stable.latest.default
          ] ++ commonBuildInputs;
        };

        defaultApp = utils.lib.mkApp {
          drv = self.defaultPackage."${system}";
        };

        devShell = with pkgs; mkShell {
          buildInputs = [
            #cargo
            cargo-insta
            pre-commit
            #rust-analyzer
            #rustPackages.clippy
            #rustc
            #rustfmt
            tokei
          ] ++ commonBuildInputs;
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LD_LIBRARY_PATH = lib.makeLibraryPath commonBuildInputs;
          GIT_EXTERNAL_DIFF = "${difftastic}/bin/difft";
        };
      });
}
