//! Host tool for the firmware's build: turns the core WebAssembly modules
//! cargo produces for the built-in apps into what the image carries of
//! each (the phone's record of the app and the code compiled for the
//! phone).  The Nix derivations and the tests use it.
//!
//! The component and the manifest are made and read by `cargo-dphone-app`,
//! the tool app makers use: there is one way from an app crate to an app.

use cargo_dphone_app::{Manifest, ManifestError, manifest::RECORD};
use clap::{Args, Parser, ValueEnum};
use dphone_apps::engine::{AppEngine, Target};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Res<T> = Result<T, String>;

/// Turn the built-in apps' WebAssembly modules into what the image carries
#[derive(Debug, Parser)]
#[command(name = "dphone-apps-build")]
enum Cli {
    /// Compile one component
    Precompile {
        /// The component (an app file)
        component: PathBuf,
        /// The compiled code to write
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
        #[arg(long, default_value = "phone")]
        target: TargetArg,
    },
    /// Lay out every app among the modules in a directory
    ///
    /// Writes ROOT/<name>/{app.pb,<name>.wasm,<name>.cwasm} for
    /// every module that is an app (it has a manifest; see dphone-app-sdk).
    Stage {
        /// The directory of modules, as cargo leaves them
        #[arg(long, value_name = "DIR")]
        wasm_dir: PathBuf,
        #[command(flatten)]
        to: Staging,
    },
    /// Run cargo for wasm32-unknown-unknown in a workspace of apps, then stage
    Build {
        /// The workspace of apps
        #[arg(long, value_name = "DIR")]
        apps: PathBuf,
        #[command(flatten)]
        to: Staging,
    },
}

#[derive(Debug, Args)]
struct Staging {
    /// Where the apps are laid out
    #[arg(long, value_name = "ROOT")]
    out: PathBuf,
    #[arg(long, default_value = "phone")]
    target: TargetArg,
    /// Leave the components out: compiled code only
    #[arg(long)]
    no_wasm: bool,
}

/// What the code is compiled for.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum TargetArg {
    /// The phone (ARMv6)
    Phone,
    /// This machine
    Host,
    /// The portable interpreter, as the tests run it
    Pulley64,
}

impl From<TargetArg> for Target {
    fn from(target: TargetArg) -> Target {
        match target {
            TargetArg::Phone => Target::Phone,
            TargetArg::Host => Target::Host,
            TargetArg::Pulley64 => Target::Pulley64,
        }
    }
}

fn read(p: &Path) -> Res<Vec<u8>> {
    std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn write(p: &Path, data: &[u8]) -> Res<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(p, data).map_err(|e| format!("{}: {e}", p.display()))
}

fn stage(wasm_dir: &Path, out: &Path, target: Target, keep_wasm: bool) -> Res<()> {
    let engine = AppEngine::new(target).map_err(|e| e.to_string())?;
    let mut modules: Vec<PathBuf> = std::fs::read_dir(wasm_dir)
        .map_err(|e| format!("{}: {e}", wasm_dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wasm"))
        .collect();
    modules.sort();
    let mut staged = 0;
    for path in modules {
        let module = read(&path)?;
        match Manifest::from_wasm(&module) {
            Ok(_) => {}
            // Not an app.
            Err(ManifestError::NoManifest) => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        // The app file, made as `cargo dphone-app` makes it for an app
        // that is installed; a built-in app is in the image as the phone
        // would keep it, with the code compiled here.
        let app =
            cargo_dphone_app::build(&module).map_err(|e| format!("{}: {e}", path.display()))?;
        let m = &app.manifest;
        let code = engine
            .precompile(&app.component)
            .map_err(|e| format!("{}: {e}", m.name))?;
        let dest = out.join(&m.name);
        let _ = std::fs::remove_dir_all(&dest);
        write(&dest.join(RECORD), &m.record())?;
        if keep_wasm {
            write(&dest.join(&m.wasm), &app.component)?;
        }
        write(&dest.join(format!("{}.cwasm", m.name)), &code)?;
        let uses: Vec<&str> = m.capabilities.iter().map(|c| c.name()).collect();
        println!(
            "{}: module {} B, component {} B, compiled {} B; uses {}",
            m.name,
            module.len(),
            app.component.len(),
            code.len(),
            uses.join(", ")
        );
        staged += 1;
    }
    if staged == 0 {
        return Err(format!("no apps in {}", wasm_dir.display()));
    }
    Ok(())
}

fn run(cli: Cli) -> Res<()> {
    match cli {
        Cli::Precompile { component, output, target } => {
            let engine = AppEngine::new(target.into()).map_err(|e| e.to_string())?;
            let code = engine
                .precompile(&read(&component)?)
                .map_err(|e| e.to_string())?;
            write(&output, &code)
        }
        Cli::Stage { wasm_dir, to } => stage(&wasm_dir, &to.out, to.target.into(), !to.no_wasm),
        Cli::Build { apps, to } => {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let target_dir = std::env::var("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| apps.join("target"));
            let status = Command::new(cargo)
                .current_dir(&apps)
                .args([
                    "build",
                    "--release",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--target-dir",
                ])
                .arg(&target_dir)
                .status()
                .map_err(|e| format!("running cargo: {e}"))?;
            if !status.success() {
                return Err("cargo build of the apps failed".into());
            }
            stage(
                &target_dir.join("wasm32-unknown-unknown/release"),
                &to.out,
                to.target.into(),
                !to.no_wasm,
            )
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dphone-apps-build: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line() {
        Cli::command().debug_assert();
        for name in ["precompile", "stage", "build"] {
            let e = Cli::try_parse_from(["dphone-apps-build", name, "--help"]).unwrap_err();
            assert_eq!(e.kind(), clap::error::ErrorKind::DisplayHelp);
            assert!(e.to_string().contains(&format!("Usage: dphone-apps-build {name}")), "{e}");
        }
        // As the Nix derivations and the tests call it.
        let Ok(Cli::Stage { to, .. }) =
            Cli::try_parse_from(["dphone-apps-build", "stage", "--wasm-dir", "m", "--out", "o", "--target", "phone", "--no-wasm"])
        else {
            panic!("stage does not parse")
        };
        assert!(matches!(to.target, TargetArg::Phone) && to.no_wasm);
        let Ok(Cli::Build { to, .. }) =
            Cli::try_parse_from(["dphone-apps-build", "build", "--target", "pulley64", "--apps", "a", "--out", "o"])
        else {
            panic!("build does not parse")
        };
        assert!(matches!(to.target, TargetArg::Pulley64) && !to.no_wasm);
        assert!(Cli::try_parse_from(["dphone-apps-build", "precompile", "c.wasm", "-o", "c.cwasm"]).is_ok());
        assert!(Cli::try_parse_from(["dphone-apps-build", "precompile", "c.wasm"]).is_err());
        assert!(Cli::try_parse_from(["dphone-apps-build", "stage", "--out", "o"]).is_err());
        assert!(Cli::try_parse_from(["dphone-apps-build", "stage", "--wasm-dir", "m", "--out", "o", "--target", "arm"]).is_err());
        assert!(Cli::try_parse_from(["dphone-apps-build"]).is_err());
    }
}
