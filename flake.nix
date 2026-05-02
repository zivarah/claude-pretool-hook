{
  description = "Configurable PreToolUse hook for Claude Code";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          claude-pretool-hook = pkgs.callPackage ./nix/claude-pretool-hook.nix { };
        in
        {
          inherit claude-pretool-hook;
          default = claude-pretool-hook;
        }
      );

      homeManagerModules.claude-pretool-hook =
        { pkgs, ... }@args:
        import ./nix/homeManagerModule.nix (
          args // { defaultPackage = self.packages.${pkgs.stdenv.hostPlatform.system}.claude-pretool-hook; }
        );

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              cargo-shear
              clippy
              rust-analyzer
              rustc
              rustfmt
            ];
          };
        }
      );
    };
}
