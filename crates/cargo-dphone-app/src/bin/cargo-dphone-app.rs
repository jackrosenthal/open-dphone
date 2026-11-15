//! `cargo dphone-app`: builds an app crate into the file the phone installs.

use cargo_dphone_app::{Manifest, ManifestError, build, componentize};
use clap::{Args, Parser, Subcommand};
use std::ffi::OsString;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const TARGET: &str = "wasm32-unknown-unknown";

/// `cargo dphone-app ...`: cargo runs `cargo-dphone-app dphone-app ...`.
#[derive(Debug, Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum Cargo {
    DphoneApp(Cli),
}

/// Build an Open DPhone app: the one file the phone installs
///
/// Without a subcommand: build.
#[derive(Debug, Args)]
#[command(version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    #[command(flatten)]
    build: Build,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    Build(Build),
    /// Make the app of a module that was built some other way
    ///
    /// This is the second step of build on its own.
    Componentize {
        /// The WebAssembly module
        module: PathBuf,
        /// The app file to write
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
    },
}

/// Build the app crate here and write its app file
///
/// Runs 'cargo build --release --target wasm32-unknown-unknown', makes a
/// WebAssembly component of each app's module, and writes
/// <target>/dphone-app/<name>.wasm, where <name> is the app's name in its
/// manifest.  An app is a cdylib crate that registers itself with
/// export_app! of dphone-app-sdk.  The file is checked as the phone checks
/// it.
#[derive(Debug, Args)]
struct Build {
    /// Write the app to this file instead
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// The app to build, in a workspace of several [default: every app]
    #[arg(short, long, value_name = "SPEC")]
    package: Vec<String>,
    /// The Cargo.toml of the app or the workspace
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,
    /// More options for 'cargo build', after '--'
    #[arg(last = true, value_name = "CARGO OPTIONS")]
    cargo_options: Vec<String>,
}

impl Build {
    /// The options for `cargo build`.
    fn cargo_options(&self) -> Vec<OsString> {
        let mut options = Vec::new();
        for package in &self.package {
            options.extend(["--package".into(), package.into()]);
        }
        if let Some(path) = &self.manifest_path {
            options.extend(["--manifest-path".into(), path.into()]);
        }
        options.extend(self.cargo_options.iter().map(OsString::from));
        options
    }
}

type Res<T> = Result<T, String>;

fn read(p: &Path) -> Res<Vec<u8>> {
    std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn write(p: &Path, data: &[u8]) -> Res<()> {
    if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(p, data).map_err(|e| format!("{}: {e}", p.display()))
}

/// Runs cargo and returns the modules of the cdylib crates it built for
/// the phone's target.
fn cargo_build(options: &[OsString]) -> Res<Vec<PathBuf>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut child = Command::new(cargo)
        .args(["build", "--release", "--target", TARGET])
        .arg("--message-format=json-render-diagnostics")
        .args(options)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("running cargo: {e}"))?;
    let mut built = Vec::new();
    let stdout = child.stdout.take().ok_or("running cargo: no output")?;
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line.map_err(|e| format!("reading cargo's output: {e}"))?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v["reason"] != "compiler-artifact" {
            continue;
        }
        let is_cdylib = v["target"]["kind"]
            .as_array()
            .is_some_and(|k| k.iter().any(|k| k == "cdylib"));
        let module = v["filenames"].as_array().and_then(|f| {
            f.iter()
                .filter_map(|f| f.as_str())
                .find(|f| f.ends_with(".wasm"))
        });
        if let (true, Some(module)) = (is_cdylib, module) {
            built.push(PathBuf::from(module));
        }
    }
    let status = child.wait().map_err(|e| format!("running cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build failed (if the target is missing: rustup target add {TARGET})"
        ));
    }
    Ok(built)
}

/// Makes the app file of one module; returns a line about it.
fn app_file(module: &[u8], out: impl FnOnce(&Manifest) -> PathBuf) -> Res<String> {
    let app = build(module)?;
    app.check_installable()?;
    let path = out(&app.manifest);
    write(&path, &app.component)?;
    let uses: Vec<&str> = app.manifest.capabilities.iter().map(|c| c.name()).collect();
    Ok(format!(
        "{} ({:?}): {} ({} bytes, {} of code; uses {})",
        app.manifest.name,
        app.manifest.display_name,
        path.display(),
        app.component.len(),
        cargo_dphone_app::manifest::code_size(&app.component),
        if uses.is_empty() {
            "no interface of the phone".to_string()
        } else {
            uses.join(", ")
        }
    ))
}

fn build_apps(build: Build) -> Res<()> {
    let options = build.cargo_options();
    let output = build.output;
    // The cdylib crates that are apps: their modules have a manifest.
    let mut apps = Vec::new();
    for path in cargo_build(&options)? {
        let module = read(&path)?;
        match Manifest::from_wasm(&module) {
            Ok(_) => apps.push((path, module)),
            Err(ManifestError::NoManifest) => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    if apps.is_empty() {
        return Err(
            "cargo built no app: an app is a crate with crate-type = [\"cdylib\"] that registers itself with dphone_app_sdk::export_app!"
                .into(),
        );
    }
    if output.is_some() && apps.len() > 1 {
        return Err("-o names one file, but cargo built several apps; choose one with -p".into());
    }
    for (path, module) in apps {
        // <target>/wasm32-unknown-unknown/release/<module>
        let target_dir = path.ancestors().nth(3).unwrap_or(Path::new("target"));
        let line = app_file(&module, |m| {
            output
                .clone()
                .unwrap_or_else(|| target_dir.join("dphone-app").join(&m.wasm))
        })
        .map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("         App {line}");
    }
    Ok(())
}

fn run(cli: Cli) -> Res<()> {
    match cli.command {
        None => build_apps(cli.build),
        Some(Cmd::Build(build)) => build_apps(build),
        Some(Cmd::Componentize { module, output }) => {
            let module = read(&module)?;
            match Manifest::from_wasm(&module) {
                Ok(_) => app_file(&module, |_| output).map(|line| eprintln!("App {line}")),
                // Not an app, e.g., a test module: only the component.
                Err(ManifestError::NoManifest) => write(&output, &componentize(&module)?),
                Err(e) => Err(e.to_string()),
            }
        }
    }
}

fn main() -> ExitCode {
    let Cargo::DphoneApp(cli) = Cargo::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cargo-dphone-app: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    fn parse(words: &[&str]) -> Result<Cli, clap::Error> {
        Cargo::try_parse_from(["cargo", "dphone-app"].iter().chain(words))
            .map(|Cargo::DphoneApp(cli)| cli)
    }

    #[test]
    fn the_command_line() {
        Cargo::command().debug_assert();
        for words in [
            &["--help"][..],
            &["build", "--help"],
            &["componentize", "--help"],
            &["help"],
        ] {
            let e = parse(words).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::DisplayHelp, "{words:?}");
            assert!(e.to_string().contains("Usage: cargo dphone-app"), "{e}");
        }
        assert_eq!(
            parse(&["--version"]).unwrap_err().kind(),
            ErrorKind::DisplayVersion
        );
    }

    /// Build is what happens without a subcommand, and takes the same
    /// options either way.
    #[test]
    fn build() {
        let cli = parse(&[]).unwrap();
        assert!(
            cli.command.is_none()
                && cli.build.output.is_none()
                && cli.build.cargo_options().is_empty()
        );
        for words in [
            &[
                "-o",
                "a.wasm",
                "-p",
                "one",
                "--manifest-path",
                "x/Cargo.toml",
                "--",
                "--offline",
                "-v",
            ][..],
            &[
                "build",
                "-o",
                "a.wasm",
                "-p",
                "one",
                "--manifest-path",
                "x/Cargo.toml",
                "--",
                "--offline",
                "-v",
            ],
        ] {
            let cli = parse(words).unwrap();
            let build = match cli.command {
                None => cli.build,
                Some(Cmd::Build(build)) => build,
                other => panic!("{other:?}"),
            };
            assert_eq!(build.output.as_deref(), Some(Path::new("a.wasm")));
            assert_eq!(
                build.cargo_options(),
                [
                    "--package",
                    "one",
                    "--manifest-path",
                    "x/Cargo.toml",
                    "--offline",
                    "-v"
                ]
            );
        }
        // An option of cargo's that is not one of ours goes after `--`.
        assert!(parse(&["--offline"]).is_err());
        assert!(parse(&["-o"]).is_err());
    }

    #[test]
    fn componentize_arguments() {
        let Ok(Cli {
            command: Some(Cmd::Componentize { module, output }),
            ..
        }) = parse(&["componentize", "m.wasm", "-o", "app.wasm"])
        else {
            panic!("componentize does not parse")
        };
        assert_eq!(
            (module.as_path(), output.as_path()),
            (Path::new("m.wasm"), Path::new("app.wasm"))
        );
        assert!(parse(&["componentize", "m.wasm"]).is_err());
        assert!(parse(&["componentize", "-o", "app.wasm"]).is_err());
        assert!(parse(&["componentize", "a.wasm", "b.wasm", "-o", "app.wasm"]).is_err());
    }
}
