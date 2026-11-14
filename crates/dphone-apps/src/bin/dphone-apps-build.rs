//! Host tool for building apps: turns the core WebAssembly modules cargo
//! produces into components, precompiles them for the phone, and packs
//! installable packages.  The Nix derivations and developers both use it.

use dphone_apps::engine::{AppEngine, Target};
use dphone_apps::manifest::Manifest;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage:
  dphone-apps-build componentize <module.wasm> -o <component.wasm>
  dphone-apps-build precompile <component.wasm> -o <out.cwasm> [--target phone|host|pulley64]
  dphone-apps-build pack <app-dir> -o <package.zip>
  dphone-apps-build stage --apps <apps-src-dir> --wasm-dir <dir-of-modules> --out <root>
                          [--target phone|host|pulley64] [--no-wasm]
  dphone-apps-build build --apps <apps-src-dir> --out <root> [--target ...] [--no-wasm]

stage lays out <root>/<name>/{app.json,<name>.wasm,<name>.cwasm,data files} for
every directory under <apps-src-dir> that has an app.json.  build runs cargo
for wasm32-unknown-unknown in <apps-src-dir> first.";

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

/// Wraps a core module built with `app-sdk` into a component.
fn componentize(module: &[u8]) -> Res<Vec<u8>> {
    wit_component::ComponentEncoder::default()
        .module(module)
        .map_err(|e| format!("{e:#}"))?
        .validate(true)
        .encode()
        .map_err(|e| format!("{e:#}"))
}

fn stage(apps: &Path, wasm_dir: &Path, out: &Path, target: Target, keep_wasm: bool) -> Res<()> {
    let engine = AppEngine::new(target).map_err(|e| e.to_string())?;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(apps)
        .map_err(|e| format!("{}: {e}", apps.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("app.json").is_file())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        return Err(format!("no apps under {}", apps.display()));
    }
    for dir in dirs {
        let text = String::from_utf8(read(&dir.join("app.json"))?).map_err(|e| e.to_string())?;
        let m = Manifest::parse(&text).map_err(|e| format!("{}: {e}", dir.display()))?;
        let stem = m.wasm.strip_suffix(".wasm").unwrap_or(&m.wasm);
        let module = read(&wasm_dir.join(format!("{stem}.wasm")))?;
        let component = componentize(&module).map_err(|e| format!("{}: {e}", m.name))?;
        let code = engine
            .precompile(&component)
            .map_err(|e| format!("{}: {e}", m.name))?;
        let dest = out.join(&m.name);
        let _ = std::fs::remove_dir_all(&dest);
        write(&dest.join("app.json"), text.as_bytes())?;
        if keep_wasm {
            write(&dest.join(&m.wasm), &component)?;
        }
        write(&dest.join(format!("{stem}.cwasm")), &code)?;
        // Package data files (icons, sounds) sit in `data/`.
        if let Ok(rd) = std::fs::read_dir(dir.join("data")) {
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            files.sort();
            for f in files {
                write(&dest.join(f.file_name().unwrap()), &read(&f)?)?;
            }
        }
        println!(
            "{}: module {} B, component {} B, compiled {} B",
            m.name,
            module.len(),
            component.len(),
            code.len()
        );
    }
    Ok(())
}

fn run(args: &[String]) -> Res<()> {
    match args.first().map(String::as_str) {
        Some("componentize") => {
            let input = args.get(1).ok_or(USAGE)?;
            write(
                Path::new(&need(args, "-o")?),
                &componentize(&read(Path::new(input))?)?,
            )
        }
        Some("precompile") => {
            let input = args.get(1).ok_or(USAGE)?;
            let engine = AppEngine::new(target(args)?).map_err(|e| e.to_string())?;
            let code = engine
                .precompile(&read(Path::new(input))?)
                .map_err(|e| e.to_string())?;
            write(Path::new(&need(args, "-o")?), &code)
        }
        Some("pack") => {
            let dir = args.get(1).ok_or(USAGE)?;
            let out = need(args, "-o")?;
            let f = std::fs::File::create(&out).map_err(|e| format!("{out}: {e}"))?;
            dphone_apps::package::pack(Path::new(dir), f).map_err(|e| e.to_string())
        }
        Some("stage") => stage(
            Path::new(&need(args, "--apps")?),
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
                &apps,
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
