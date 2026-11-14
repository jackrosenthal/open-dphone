//! Developer conveniences that are not builds.  Each subcommand lives in its
//! own module; add new ones to the table in `main`.

mod kernel;

use std::process::ExitCode;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Command {
    name: &'static str,
    usage: &'static str,
    run: fn(&[String]) -> Result<()>,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "kernel-sync",
        usage: "kernel-sync <tree>            regenerate kernel/{patches,files,configs} from a v7.3 working tree",
        run: kernel::sync,
    },
    Command {
        name: "kernel-prepare",
        usage: "kernel-prepare <tree>         apply kernel/{patches,files,configs} to a pristine v7.3 tree",
        run: kernel::prepare,
    },
    Command {
        name: "kernel-build",
        usage: "kernel-build <tree> <out>     build zImage, DTBs, and modules; collect per-model images in <out>",
        run: kernel::build,
    },
];

fn usage() -> ExitCode {
    eprintln!("usage: cargo xtask <command> [args]\n");
    for c in COMMANDS {
        eprintln!("  {}", c.usage);
    }
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first() else {
        return usage();
    };
    let Some(cmd) = COMMANDS.iter().find(|c| c.name == name) else {
        return usage();
    };
    match (cmd.run)(&args[1..]) {
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
