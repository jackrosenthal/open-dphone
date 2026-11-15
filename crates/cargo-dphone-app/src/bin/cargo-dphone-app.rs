//! `cargo dphone-app`: builds an app crate into the file the phone installs.

use cargo_dphone_app::{Manifest, ManifestError, build, componentize};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const TARGET: &str = "wasm32-unknown-unknown";

const USAGE: &str = "\
Builds an Open DPhone app: the one file the phone installs.

usage:
  cargo dphone-app [build] [-o <app.wasm>] [cargo build options]
  cargo dphone-app componentize <module.wasm> -o <app.wasm>

build (the default) runs 'cargo build --release --target wasm32-unknown-unknown'
with the options given (e.g., --manifest-path, -p), makes a WebAssembly
component of each app's module, and writes <target>/dphone-app/<name>.wasm
(or the file given with -o), where <name> is the app's name in its
manifest.  An app is a cdylib crate that registers itself with export_app!
of dphone-app-sdk.  The file is checked as the phone checks it.

componentize is the second step on its own, for a module built some other
way.";

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

/// Takes `-o <file>` out of the arguments.
fn take_output(args: &mut Vec<String>) -> Res<Option<PathBuf>> {
    let Some(i) = args.iter().position(|a| a == "-o" || a == "--output") else {
        return Ok(None);
    };
    if i + 1 >= args.len() {
        return Err("-o needs a file name".into());
    }
    args.remove(i);
    Ok(Some(PathBuf::from(args.remove(i))))
}

/// Runs cargo and returns the modules of the cdylib crates it built for
/// the phone's target.
fn cargo_build(options: &[String]) -> Res<Vec<PathBuf>> {
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
        "{} ({:?}): {} ({} bytes{}; uses {})",
        app.manifest.name,
        app.manifest.display_name,
        path.display(),
        app.component.len(),
        match app.files.len() {
            0 => String::new(),
            n => format!(", {n} data files"),
        },
        if uses.is_empty() {
            "no interface of the phone".to_string()
        } else {
            uses.join(", ")
        }
    ))
}

fn build_apps(mut options: Vec<String>) -> Res<()> {
    let output = take_output(&mut options)?;
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

fn run(mut args: Vec<String>) -> Res<()> {
    // As a cargo subcommand the first argument is the subcommand's name.
    if args.first().is_some_and(|a| a == "dphone-app") {
        args.remove(0);
    }
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("cargo-dphone-app {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("componentize") => {
            let mut rest = args.split_off(1);
            let out = take_output(&mut rest)?.ok_or(USAGE)?;
            let [module] = rest.as_slice() else {
                return Err(USAGE.into());
            };
            let module = read(Path::new(module))?;
            match Manifest::from_wasm(&module) {
                Ok(_) => app_file(&module, |_| out).map(|line| eprintln!("App {line}")),
                // Not an app, e.g., a test module: only the component.
                Err(ManifestError::NoManifest) => write(&out, &componentize(&module)?),
                Err(e) => Err(e.to_string()),
            }
        }
        Some("build") => build_apps(args.split_off(1)),
        _ => build_apps(args),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cargo-dphone-app: {e}");
            ExitCode::FAILURE
        }
    }
}
