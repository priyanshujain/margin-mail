{
  description = "Margin Mail, a calm, keyboard-first mail client";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      overlays.default = final: prev: {
        margin-mail = final.callPackage ./nix/package.nix { };
      };

      packages.${system} = {
        margin-mail = pkgs.callPackage ./nix/package.nix { };
        default = self.packages.${system}.margin-mail;
      };
    };
}
