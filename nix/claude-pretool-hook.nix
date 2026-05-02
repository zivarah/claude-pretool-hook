{ lib, rustPlatform }:
rustPlatform.buildRustPackage rec {
  pname = "claude-pretool-hook";

  src =
    let
      fs = lib.fileset;
    in
    fs.toSource {
      root = ../.;
      fileset = fs.gitTracked ../.;
    };

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  version = lib.head (
    lib.splitString "\"" (
      lib.head (lib.tail (lib.splitString "version = \"" (builtins.readFile ../Cargo.toml)))
    )
  );

  meta = {
    mainProgram = "claude-pretool-hook";
    description = "Configurable PreToolUse hook for Claude Code";
  };
}
