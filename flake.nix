{
  description = "Open DPhone: open firmware for the Digium D-series phones";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Builds cargo dependencies in their own derivation (nix/workspace.nix).
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
        # The proprietary inputs, which whoever builds supplies: the DSP
        # firmware for the images (nix/dsp-firmware.nix) and the stock
        # firmware images for the checks (nix/vendor-firmware.nix).  Nothing
        # else unfree is allowed.
        config.allowUnfreePredicate =
          pkg:
          let
            name = nixpkgs.lib.getName pkg;
          in
          nixpkgs.lib.hasPrefix "dspapp1107_lcc" name || nixpkgs.lib.hasSuffix "_firmware.eff" name;
      };
      lib = pkgs.lib;
      od = import ./nix {
        inherit pkgs lib crane;
      };
    in
    {
      packages.${system} = od.packages;
      checks.${system} = od.checks;
      devShells.${system}.default = od.devShell;
      formatter.${system} = pkgs.nixfmt;
    };
}
