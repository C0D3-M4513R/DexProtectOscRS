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
          gtk4
          pkg-config

          libGL
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          xorg.libxcb
          fontconfig
        ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isUnix [
					libxkbcommon
				] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
					wayland
 				];
        runtimeDependencies = with pkgs; [
          libGL
        ]++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isUnix [
					libxkbcommon
				] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
        	wayland
          libappindicator
        ];
        app = pkgs.rustPlatform.buildRustPackage {
					name = manifest.name;
					pversion = manifest.version;

					src = pkgs.lib.cleanSource ./.;
					cargoLock = {
						lockFile = ./Cargo.lock;
						outputHashes = {
								 "muda-0.17.1" = "sha256-eY8IsAyZIWtNltP8q+Zqb/4pt3QOVbNPyLPYKi6lqfE=";
								 "tray-icon-0.21.3" = "sha256-P3mKX5ciOLdDg6Kr1ZdXZOKsyptIAFvHr2pL8iiGqjY=";
						};
					};
					doCheck = true;

					nativeBuildInputs = [
						pkgs.autoPatchelfHook
						pkgs.wrapGAppsHook3
						pkgs.copyDesktopItems
						pkgs.rust-bin.stable.latest.minimal
						pkgs.pkg-config
					];

					runtimeDependencies = runtimeDependencies;

					buildInputs = with pkgs; [
					] ++ commonBuildInputs;

				  #FIXME(tray-icon): Darwin has is known broken compilation for tray-icon: https://github.com/tauri-apps/tray-icon/pull/201#issuecomment-3679434001
					buildFeatures = [] ++ pkgs.lib.optionals (!pkgs.stdenv.hostPlatform.isDarwin) ["tray"];

					desktopItems =
					let
					  item = pkgs.makeDesktopItem {
							name = manifest.name;
							desktopName = "DexProtectOscRs";
							exec = manifest.name;
							categories = [
								"Utility"
							];
							icon = "dex_protect_osc_rs";
						};
					in [ item ];

					postInstall = ''
						mv images/app.png images/dex_protect_osc_rs.png
						install -Dm644 -t $out/share/icons images/dex_protect_osc_rs.png
					'';

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
						#
						#If you wish to use this with the key present, I currently use it in my system flake like this: (where dexprotect is a flake input to this repo and included via special args)
						#      (dexprotext.overrideAttrs (finalAttrs: previousAttrs: {
            #        postPatch = ''
            #          echo '${dex_key}' > 'app/src/osc/dex_key.rs'
            #        '';
            #      }))
#						license = pkgs.lib.licenses.unfreeRedistributable; #Technically this is unfree redistributable, but I don't wanna build my nixos impure every-time.
						platforms = pkgs.lib.platforms.unix ++ pkgs.lib.platforms.windows ++ pkgs.lib.platforms.darwin;
						mainProgram = manifest.name;
					};
				};
      in
      {
				packages = {
					default = app;
				};

				apps = rec{
					default = utils.lib.mkApp {
						drv = app;
					};
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
            #windows cross-building
            pkgsCross.mingwW64.stdenv.cc
            pkgsCross.mingwW64.windows.pthreads
          ] ++ commonBuildInputs ++ runtimeDependencies ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
						heaptrack
            #linux tray stuff
            xdotool
					];
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LD_LIBRARY_PATH = lib.makeLibraryPath (commonBuildInputs ++ runtimeDependencies);
          GIT_EXTERNAL_DIFF = "${difftastic}/bin/difft";
        };
      });
}
