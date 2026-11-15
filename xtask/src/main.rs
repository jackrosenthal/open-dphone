//! Developer conveniences that are not builds.  Each subcommand lives in its
//! own module; add new ones to [`Xtask`].

mod deploy;
mod kernel;
mod tz;

use std::process::ExitCode;

use clap::Parser;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Developer conveniences that are not builds
#[derive(Debug, Parser)]
#[command(name = "xtask", bin_name = "cargo xtask")]
enum Xtask {
    /// Build dphone, push it to /tmp/dphone on a phone, and restart the main role
    Deploy(deploy::Deploy),
    /// Regenerate the phone's time zone rule table from a zone database
    TzTable(tz::TzTable),
    /// Regenerate kernel/{patches,files,configs} from a v7.3 working tree
    KernelSync(kernel::KernelSync),
}

fn main() -> ExitCode {
    let (name, result) = match Xtask::parse() {
        Xtask::Deploy(args) => ("deploy", deploy::deploy(args)),
        Xtask::TzTable(args) => ("tz-table", tz::table(args)),
        Xtask::KernelSync(args) => ("kernel-sync", kernel::sync(args)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask {name}: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The repository root (the directory holding `xtask/`).
fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent directory")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line() {
        Xtask::command().debug_assert();
        for name in ["deploy", "tz-table", "kernel-sync"] {
            let e = Xtask::try_parse_from(["xtask", name, "--help"]).unwrap_err();
            assert_eq!(e.kind(), clap::error::ErrorKind::DisplayHelp);
            assert!(e.to_string().contains(&format!("Usage: cargo xtask {name}")), "{e}");
        }
        assert!(matches!(Xtask::try_parse_from(["xtask", "deploy", "10.0.0.5", "--no-build"]), Ok(Xtask::Deploy(_))));
        assert!(matches!(Xtask::try_parse_from(["xtask", "tz-table"]), Ok(Xtask::TzTable(_))));
        assert!(matches!(Xtask::try_parse_from(["xtask", "kernel-sync", "/src/linux"]), Ok(Xtask::KernelSync(_))));
        assert!(Xtask::try_parse_from(["xtask"]).is_err());
        assert!(Xtask::try_parse_from(["xtask", "deploy"]).is_err());
        assert!(Xtask::try_parse_from(["xtask", "kernel-sync"]).is_err());
        assert!(Xtask::try_parse_from(["xtask", "kernel-sync", "a", "b"]).is_err());
    }
}
