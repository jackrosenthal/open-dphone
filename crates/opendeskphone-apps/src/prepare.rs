//! Checking and compiling an app off the manager's thread.
//!
//! The phone compiles an installed app's component to Pulley bytecode
//! itself (an app file never carries compiled code: Wasmtime trusts what
//! it deserializes).  Measured on a D60 (ARM1176, 2026-11-15), that takes
//! about 2 s plus 0.12 s for every KiB of the component: 5 s for a small
//! app, 16 s for the 119 KiB Light Up.  The manager's thread cannot stop
//! for that long (the running apps would stand still, and the liveness
//! check would call the subsystem dead), so a worker thread does it, at
//! the lowest priority, one app at a time, and hands the result back.

use crate::engine::AppEngine;
use crate::host::{self, AppCtx};
use crate::install::{AppFile, InstallError, Installer};
use crate::manifest::Capability;
use std::path::{Path, PathBuf};
use wasmtime::component::Linker;

/// File holding code this phone compiled for an installed app.
pub(crate) const CODE_CACHE: &str = ".code.cwasm";

/// The most code (the code sections of the component's modules) the phone
/// compiles: about three minutes of compiling by the figures above.  The
/// largest built-in app has about a tenth of this.  An app's data, which
/// is in its file too, does not count: the compiler only copies it.
pub const MAX_COMPONENT: u64 = 1 << 20;

/// A compilation for the worker thread.
#[derive(Clone, Debug)]
pub(crate) enum Job {
    /// An app file to check and compile (the command `InstallApp`).
    Install { path: PathBuf },
    /// An installed app whose compiled code this firmware cannot load.
    Recompile { name: String, wasm: PathBuf },
}

/// An app file that passed every check, with its compiled code.
pub(crate) struct Prepared {
    pub package: AppFile,
    pub code: Vec<u8>,
}

/// What the worker thread hands back.
pub(crate) enum Outcome {
    Install(Result<Box<Prepared>, InstallError>),
    Recompile(Result<Vec<u8>, String>),
}

fn compile(engine: &AppEngine, name: &str, wasm: &[u8]) -> Result<Vec<u8>, String> {
    let started = std::time::Instant::now();
    let code = engine.precompile(wasm).map_err(|e| e.to_string())?;
    log::info!(
        "app {name}: compiled {} bytes of WebAssembly to {} bytes in {} ms",
        wasm.len(),
        code.len(),
        started.elapsed().as_millis()
    );
    Ok(code)
}

/// The interfaces of the phone a component imports: an app's capabilities
/// are what it uses, not what it says.
pub fn capabilities(
    engine: &AppEngine,
    component: &wasmtime::component::Component,
) -> Result<Vec<Capability>, crate::ManifestError> {
    let mut found = Vec::new();
    for (import, _) in component.component_type().imports(engine.wasmtime()) {
        if let Some(c) = Capability::of_import(import)?
            && !found.contains(&c)
        {
            found.push(c);
        }
    }
    Ok(found)
}

/// Reads an app file, checks everything that can be checked without
/// touching the installed apps, and compiles its component.
pub(crate) fn prepare(
    engine: &AppEngine,
    installer: &Installer,
    path: &Path,
    required_name: Option<&str>,
) -> Result<Prepared, InstallError> {
    let mut package = AppFile::open(path)?;
    let size = package.code_size();
    if size > MAX_COMPONENT {
        return Err(InstallError::ComponentTooLarge {
            size,
            max: MAX_COMPONENT,
        });
    }
    // Before the compilation, which is the long part.
    installer.check(&package, required_name, 0)?;
    // Compiling proves the component valid and yields the code the phone
    // will run.
    let code = compile(engine, &package.manifest.name, &package.component)
        .map_err(InstallError::Unloadable)?;
    // SAFETY: `code` is what this engine just compiled.
    let component = unsafe { engine.load_compiled(&code) }
        .map_err(|e| InstallError::Unloadable(e.to_string()))?;
    // What the app may use is what its component imports.
    let capabilities = capabilities(engine, &component)?;
    package.manifest.capabilities = capabilities;
    let mut linker = Linker::<AppCtx>::new(engine.wasmtime());
    host::link(&mut linker, &package.manifest)
        .and_then(|_| linker.instantiate_pre(&component).map(|_| ()))
        .map_err(|e| InstallError::Unloadable(format!("{e:#}")))?;
    Ok(Prepared { package, code })
}

/// Writes an installed app's compiled code beside its component, whole or
/// not at all.
pub(crate) fn write_code(dir: &Path, code: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = dir.join(".code.new");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(code)?;
    f.sync_all()?;
    std::fs::rename(&tmp, dir.join(CODE_CACHE))
}

/// The lowest priority for the calling thread: the compiler runs only when
/// nothing else wants the processor (calls, the screen, the web interface).
fn lowest_priority() {
    #[cfg(target_os = "linux")]
    {
        // On Linux the nice value belongs to the thread.
        let _ = rustix::process::setpriority_process(Some(rustix::thread::gettid()), 19);
    }
}

/// Runs `job` on a thread of its own and calls `done` with the result.
pub(crate) fn spawn(
    engine: AppEngine,
    installer: Installer,
    job: Job,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("app-compile".into())
        // Cranelift recurses over the function it compiles.
        .stack_size(2 << 20)
        .spawn(move || {
            lowest_priority();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &job {
                Job::Install { path } => {
                    Outcome::Install(prepare(&engine, &installer, path, None).map(Box::new))
                }
                Job::Recompile { name, wasm } => Outcome::Recompile(
                    std::fs::read(wasm)
                        .map_err(|e| format!("{}: {e}", wasm.display()))
                        .and_then(|bytes| compile(&engine, name, &bytes)),
                ),
            }))
            .unwrap_or_else(|_| {
                let failed = "the compiler failed".to_string();
                match &job {
                    Job::Install { .. } => Outcome::Install(Err(InstallError::Unloadable(failed))),
                    Job::Recompile { .. } => Outcome::Recompile(Err(failed)),
                }
            });
            done(outcome);
        })
        .map(|_| ())
}
