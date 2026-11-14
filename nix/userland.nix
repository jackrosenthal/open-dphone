# The shell and the core utilities: brush and uutils coreutils, both Rust,
# both static ARMv6 binaries with no C in them (spike S7).
{
  lib,
  fetchCrate,
  rustPlatform,
  rust,
}:
let
  # Applets in the multi-call coreutils binary.  Chosen for a phone: file
  # handling, text inspection, checksums for update debugging, and the few
  # system queries that have no `dphone` subcommand.  Mounting, networking,
  # and module loading are `dphone` subcommands, not coreutils.
  applets = [
    "base64"
    "basename"
    "cat"
    "chmod"
    "chown"
    "cksum"
    "cp"
    "cut"
    "date"
    "dd"
    "df"
    "dirname"
    "du"
    "echo"
    "env"
    "expr"
    "false"
    "head"
    "hostname"
    "id"
    "ln"
    "ls"
    "md5sum"
    "mkdir"
    "mknod"
    "mktemp"
    "mv"
    "od"
    "printenv"
    "printf"
    "pwd"
    "readlink"
    "realpath"
    "rm"
    "rmdir"
    "seq"
    "sha256sum"
    "sleep"
    "sort"
    "stat"
    "stty"
    "sync"
    "tail"
    "tee"
    "test"
    "touch"
    "tr"
    "true"
    "truncate"
    "uname"
    "uniq"
    "wc"
    "yes"
  ];

  crate =
    {
      pname,
      version,
      srcHash,
      vendorHash,
      cargoArgs ? [ ],
      bin,
    }:
    let
      src = fetchCrate {
        inherit pname version;
        hash = srcHash;
      };
    in
    rust.buildCargo {
      inherit pname version src;
      cargoDeps = rustPlatform.fetchCargoVendor {
        inherit pname version src;
        hash = vendorHash;
      };
      target = rust.armTarget;
      env = rust.sizeProfileEnv;
      commands = [
        (
          [
            "build"
            "--release"
          ]
          ++ cargoArgs
        )
      ];
      bins = [ bin ];
      meta.license = lib.licenses.mit;
    };
in
{
  inherit applets;

  brush = crate {
    pname = "brush-shell";
    version = "0.4.0";
    srcHash = "sha256-XkwEa86Rn7WACUxD+BrgoiEG/CBjYyh2giTAhqyJOsU=";
    vendorHash = "sha256-7VtzTLqnPp7zdXEcCZRTqDyUh3iXhiATb/uAadF6tx0=";
    bin = "brush";
  };

  coreutils = crate {
    pname = "coreutils";
    version = "0.12.0";
    srcHash = "sha256-PLeCAp+Lat2LhL66B9FM9j7/pf77Qheb9wFB9LIWmyw=";
    vendorHash = "sha256-LROC0h9frPGNPeIIAC5Y4iy34Igb7lTVw2jcT79zuzo=";
    cargoArgs = [
      "--no-default-features"
      "--features"
      (lib.concatStringsSep "," applets)
    ];
    bin = "coreutils";
  };
}
