{
  inputs = {
    # This must be the stable nixpkgs if you're running the app on a
    # stable NixOS install.  Mixing EGL library versions doesn't work.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    utils.url = "github:numtide/flake-utils";
    rust-overlay = {
    	url = "github:oxalica/rust-overlay";
    	inputs.nixpkgs.follows = "nixpkgs";
    };
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
						 "egui_tracing-0.2.6" = "sha256-30n161ux80D+HAxJJqjgPJt/s2W3yPBRXBd1JyiVwZI=";
						};
					};
          doCheck = true;

          nativeBuildInputs = [
            pkgs.autoPatchelfHook
            pkgs.wrapGAppsHook3
          ];

          runtimeDependencies = runtimeDependencies;

          buildInputs = with pkgs; [
          ] ++ commonBuildInputs;


					meta = {
						description = "Open-Source Implementation of the accompanying app for DexProtect";
						#Nothing in this repo states, that this is the case, but you are free to redistribute compiled versions of this source code.
						#But also note, that the not-included app/src/osc/dex_key.rs file is not to be distributed at all (and is therefore not included in the source-code)!
						#The same goes for the actual IV and KEY contained within that file in ANY WAY SHAPE OR FORM.
						#This restriction originates from the DexProtect Creator, which asked me (and likely in sentiment also others who discover this information) to abide by this.
						#
						#As a special case, you are allowed to reformat the contents of app/src/osc/dex_key.rs as you wish/need
						#Note that you can redistribute versions of this app, which were compiled with app/src/osc/dex_key.rs present and without it present.
						#Though without app/src/osc/dex_key.rs present you might want to activate the no_decryption_keys feature to fix the compilation errors.
						license = pkgs.lib.licenses.unfreeRedistributable;
						platforms = pkgs.lib.platforms.linux ++ pkgs.lib.platforms.windows ++ pkgs.lib.platforms.darwin;
						mainProgram = "dex_protect_osc_rs";
					};
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
