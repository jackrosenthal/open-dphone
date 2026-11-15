//! Host tool for the firmware's build: turns the core WebAssembly modules
//! cargo produces for the built-in apps into what the image carries of
//! each (the phone's record of the app, the code compiled for the phone,
//! and the app's data files).  The Nix derivations and the tests use it.
//!
//! The component and the manifest are made and read by `cargo-dphone-app`,
//! the tool app makers use: there is one way from an app crate to an app.

use cargo_dphone_app::{Manifest, ManifestError, manifest::RECORD};
use dphone_apps::engine::{AppEngine, Target};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage:
  dphone-apps-build precompile <component.wasm> -o <out.cwasm> [--target phone|host|pulley64]
  dphone-apps-build stage --wasm-dir <dir-of-modules> --out <root>
                          [--target phone|host|pulley64] [--no-wasm]
  dphone-apps-build build --apps <apps-src-dir> --out <root> [--target ...] [--no-wasm]

stage lays out <root>/<name>/{app.pb,<name>.wasm,<name>.cwasm,data files} for
every module in <dir-of-modules> that is an app (it has a manifest; see
dphone-app-sdk).  build runs cargo for wasm32-unknown-unknown in
<apps-src-dir> first.";

type Res<T> = Result<T, String>;

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn need(args: &[String], flag: &str) -> Res<String> {
    arg_after(args, flag).ok_or_else(|| format!("missing {flag}\n{USAGE}"))
}

fn target(args: &[String]) -> Res<Target> {
    match arg_after(args, "--target").as_deref() {
        None | Some("phone") => Ok(Target::Phone),
        Some("host") => Ok(Target::Host),
        Some("pulley64") => Ok(Target::Pulley64),
        Some(t) => Err(format!("unknown target {t:?}")),
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
        for (name, data) in &app.files {
            write(&dest.join(name), data)?;
        }
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

fn run(args: &[String]) -> Res<()> {
    match args.first().map(String::as_str) {
        Some("precompile") => {
            let input = args.get(1).ok_or(USAGE)?;
            let engine = AppEngine::new(target(args)?).map_err(|e| e.to_string())?;
            let code = engine
                .precompile(&read(Path::new(input))?)
                .map_err(|e| e.to_string())?;
            write(Path::new(&need(args, "-o")?), &code)
        }
        Some("stage") => stage(
            Path::new(&need(args, "--wasm-dir")?),
            Path::new(&need(args, "--out")?),
            target(args)?,
            !args.iter().any(|a| a == "--no-wasm"),
        ),
        Some("build") => {
            let apps = PathBuf::from(need(args, "--apps")?);
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
                Path::new(&need(args, "--out")?),
                target(args)?,
                !args.iter().any(|a| a == "--no-wasm"),
            )
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dphone-apps-build: {e}");
            ExitCode::FAILURE
        }
    }
}
