{
  inputs = {
    naersk.url = "github:nmattia/naersk/master";
    # This must be the stable nixpkgs if you're running the app on a
    # stable NixOS install.  Mixing EGL library versions doesn't work.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-23.05";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-compat = {
      url = github:edolstra/flake-compat;
      flake = false;
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
        libPath = with pkgs; lib.makeLibraryPath [
          libGL
          libxkbcommon
          wayland
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          fontconfig
        ];
        buildInputs = with pkgs; [
          pkgs.rust-bin.stable.latest.default
          xorg.libxcb
          pkgs.pkg-config
          pkgs.glib
          pkgs.gtk3
          pkgs.gcc
        ];
  
      in
      {
        defaultPackage = naersk-lib.buildPackage {
          src = pkgs.lib.cleanSource ./.;
          doCheck = true;
          pname = manifest.name;
          nativeBuildInputs = [ pkgs.makeWrapper ];
          buildInputs = buildInputs;
          cargoBuildOptions = defaultOptions: defaultOptions ++ ["--features file_dialog"];
          postInstall = ''
            wrapProgram "$out/bin/$pname" --prefix LD_LIBRARY_PATH : "${libPath}"
          '';
        };

        defaultApp = utils.lib.mkApp {
          drv = self.defaultPackage."${system}";
        };

        devShell = with pkgs; mkShell {
          buildInputs = buildInputs ++ [
            #cargo
            cargo-insta
            pre-commit
            #rust-analyzer
            #rustPackages.clippy
            #rustc
            #rustfmt
            tokei

            xorg.libxcb
          ];
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LD_LIBRARY_PATH = libPath;
          GIT_EXTERNAL_DIFF = "${difftastic}/bin/difft";
        };
      });
}