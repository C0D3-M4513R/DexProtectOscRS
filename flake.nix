{
  inputs = {
    naersk.url = "github:nmattia/naersk/master";
    # This must be the stable nixpkgs if you're running the app on a
    # stable NixOS install.  Mixing EGL library versions doesn't work.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-23.05";
    utils.url = "github:numtide/flake-utils";
    flake-compat = {
      url = github:edolstra/flake-compat;
      flake = false;
    };
  };

  outputs = { self, nixpkgs, utils, naersk, ... }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        naersk-lib = pkgs.callPackage naersk { };
        manifest = (pkgs.lib.importTOML ./app/Cargo.toml).package;
        libPath = with pkgs; lib.makeLibraryPath [
          libGL
          libxkbcommon
          wayland
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
        ];
      in
      {
        defaultPackage = naersk-lib.buildPackage {
          src = pkgs.lib.cleanSource ./.;
          doCheck = true;
          pname = manifest.name;
          nativeBuildInputs = [ pkgs.makeWrapper ];
          buildInputs = with pkgs; [
            xorg.libxcb
          ];
          postInstall = ''
            wrapProgram "$out/bin/sixty-two" --prefix LD_LIBRARY_PATH : "${libPath}"
          '';
        };

        defaultApp = utils.lib.mkApp {
          drv = self.defaultPackage."${system}";
        };

        devShell = with pkgs; mkShell {
          buildInputs = [
            cargo
            cargo-insta
            pre-commit
            rust-analyzer
            rustPackages.clippy
            rustc
            rustfmt
            tokei

            xorg.libxcb
          ];
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LD_LIBRARY_PATH = libPath;
          GIT_EXTERNAL_DIFF = "${difftastic}/bin/difft";
        };
      });
}
