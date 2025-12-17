{
  inputs = {
    # This must be the stable nixpkgs if you're running the app on a
    # stable NixOS install.  Mixing EGL library versions doesn't work.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-compat = {
      url = github:edolstra/flake-compat;
      flake = true;
    };
  };

  outputs = { self, nixpkgs, utils, rust-overlay, ... }:
    utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {inherit system overlays;};
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
        	wayland
          libGL
          libxkbcommon
        ];
      in
      {
        defaultPackage = pkgs.rustPlatform.buildRustPackage {
          name = manifest.name;
          pversion = manifest.version;

          src = pkgs.lib.cleanSource ./.;
					cargoLock = {
						lockFile = ./Cargo.lock;
						outputHashes = {
						 "egui_tracing-0.2.2" = "sha256-PXFZVRS3y13MdvU/3IZKpgLsdmb+3TbfhXiprsWtIuw=";
						};
					};
          doCheck = true;

          nativeBuildInputs = [
            pkgs.autoPatchelfHook
            pkgs.wrapGAppsHook
          ];

          runtimeDependencies = runtimeDependencies;

          buildInputs = with pkgs; [
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
