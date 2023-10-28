with import <nixpkgs>
{
  overlays = [
    (import (fetchTarball "https://github.com/oxalica/rust-overlay/archive/master.tar.gz"))
  ];
};

let 
  manifest = (pkgs.lib.importTOML ./app/Cargo.toml).package;
  rustPlatform = makeRustPlatform {
    cargo = pkgs.rust-bin.stable.latest.default;
    rustc = pkgs.rust-bin.stable.latest.default;
  };
in
rustPlatform.buildRustPackage rec {
  pname = manifest.name;
  version = manifest.version;
  cargoLock.lockFile = ./Cargo.lock;
  cargoLock.outputHashes = {
    "egui_tracing-0.2.1" = "sha256-MWR5R5VwA7M+fkIU4vF01GsofMEYZ3IuQAbxVHSVWVY=";
  };

  src = pkgs.lib.cleanSource ./.;
  nativeBuildInputs = [ pkgs.makeWrapper ];
  buildFeatures = ["file_dialog"];
  buildInputs = [ 
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ];
  libPath = with pkgs; lib.makeLibraryPath [
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ];
  LD_LIBRARY_PATH = libPath;
  postInstall = ''
    wrapProgram "$out/bin/$pname" --prefix LD_LIBRARY_PATH : "${libPath}"
  '';
}

