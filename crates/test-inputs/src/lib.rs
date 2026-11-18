//! Where the tests find what is not in the repository: the directory
//! `test-inputs/` at the top of the repository, which is a link to the Nix
//! output of the same name and is not in git.  To make it:
//!
//! ```text
//! nix build .#test-inputs -o test-inputs
//! ```
//!
//! | Name | What |
//! |---|---|
//! | `vendor` | The stock firmware: a directory for each model, with its image and its root file system image (`D60/2_9_28_D60_firmware.eff`, `D60/rootfs.jffs2`) |
//! | `stock-rootfs` | The stock D60 root file system as a directory |
//! | `asterisk` | Asterisk: `bin/asterisk`, its sample configuration in `etc/asterisk`, and its data in `var/lib/asterisk` |
//! | `asterisk-modules` | A module directory with `res_open_dpma.so` |
//! | `apps-staged` | The built-in apps and the test guests, staged for the build machine (`opendeskphone-apps-build stage --target pulley64`) |
//!
//! A test whose input, or a facility of the host it needs (a program,
//! unprivileged namespaces), is missing prints a notice and passes.  With
//! the feature `required`, which the Nix check turns on, it fails instead,
//! so that the check cannot pass by not running it.

use std::path::{Path, PathBuf};

/// Whether a test may not skip itself (the feature `required`).
pub const REQUIRED: bool = cfg!(feature = "required");

/// The `test-inputs` directory of the repository.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-inputs")
}

/// The input `name` (see the table above), if it is there.  Otherwise
/// [`unavailable`] decides: `None`, or a failure.
pub fn input(name: &str) -> Option<PathBuf> {
    let path = dir().join(name);
    if path.exists() {
        return Some(path);
    }
    unavailable(&format!(
        "test-inputs/{name} is missing (nix build .#test-inputs -o test-inputs makes it)"
    ));
    None
}

/// Report that a test cannot run here because of `why`: a notice, or, with
/// [`REQUIRED`], a failure.
pub fn unavailable(why: &str) {
    if REQUIRED {
        panic!("{why}, and the tests are built with test-inputs/required");
    }
    eprintln!("SKIPPED: {why}");
}
