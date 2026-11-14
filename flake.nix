{
  description = "Open DPhone: open firmware for the Digium D-series phones";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # dpma-proto lives here.  To build against a local checkout:
    #   --override-input dpma-rs path:/path/to/dpma-rs
    dpma-rs = {
      url = "git+ssh://git@github.com/jackrosenthal/dpma-rs";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      dpma-rs,
    }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
        # The one proprietary input: the DSP firmware the builder supplies
        # (nix/dsp-firmware.nix).  Nothing else unfree is allowed.
        config.allowUnfreePredicate = pkg: nixpkgs.lib.hasPrefix "dspapp1107_lcc" (nixpkgs.lib.getName pkg);
      };
      lib = pkgs.lib;
      od = import ./nix {
        inherit pkgs lib dpma-rs;
      };
    in
    {
      packages.${system} = od.packages;
      checks.${system} = od.checks;
      devShells.${system}.default = od.devShell;
      formatter.${system} = pkgs.nixfmt;
    };
}
