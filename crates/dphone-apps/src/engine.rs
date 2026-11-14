//! The WebAssembly engine: Wasmtime configured for the Pulley interpreter
//! with settings sized for a 128 MiB phone.
//!
//! Built-in apps are compiled to Pulley bytecode at build time
//! (`.cwasm`, by `dphone-apps-build`) and only deserialized on the phone.
//! With the `compile` feature the engine can also compile a `.wasm`
//! component itself; the phone uses that for installed apps, caching the
//! result.
//!
//! The settings below are baked into compiled code, so the build host and
//! the phone must use the same ones; a mismatch makes deserialization fail
//! cleanly, never run wrong code.

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wasmtime::component::Component;

/// One epoch tick.  Guest call budgets are counted in these.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub enum EngineError {
    /// This build has no compiler and the app is not precompiled.
    NoCompiler,
    Wasmtime(String),
    Io(std::io::Error),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::NoCompiler => {
                write!(
                    f,
                    "this build cannot compile WebAssembly; the app must be precompiled"
                )
            }
            EngineError::Wasmtime(e) => write!(f, "{e}"),
            EngineError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::Io(e)
    }
}

fn wt(e: impl fmt::Display) -> EngineError {
    EngineError::Wasmtime(format!("{e:#}"))
}

/// Which code the engine produces and runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Whatever runs on this machine: Pulley on the phone, native code or
    /// Pulley on a workstation.
    Host,
    /// Pulley bytecode for the phone (32-bit little-endian), for
    /// precompiling on the build host.  Such an engine cannot run code on
    /// a 64-bit host.
    Phone,
    /// Pulley on a 64-bit host, to exercise the interpreter in tests.
    Pulley64,
}

fn config(target: Target) -> Result<wasmtime::Config, EngineError> {
    let mut c = wasmtime::Config::new();
    c.epoch_interruption(true);
    // No large address-space reservations: 32-bit address space, and many
    // small instances.
    c.memory_reservation(0);
    c.memory_guard_size(0);
    c.memory_reservation_for_growth(64 << 10);
    c.memory_init_cow(false);
    c.max_wasm_stack(256 << 10);
    c.wasm_simd(false);
    c.wasm_relaxed_simd(false);
    match target {
        Target::Host => {}
        #[cfg(feature = "compile")]
        Target::Phone => {
            c.target("pulley32").map_err(wt)?;
        }
        #[cfg(feature = "compile")]
        Target::Pulley64 => {
            c.target("pulley64").map_err(wt)?;
        }
        #[cfg(not(feature = "compile"))]
        _ => return Err(EngineError::NoCompiler),
    }
    #[cfg(feature = "compile")]
    {
        c.cranelift_opt_level(wasmtime::OptLevel::SpeedAndSize);
    }
    Ok(c)
}

struct Ticker {
    stop: AtomicBool,
}

/// A shared engine with its epoch ticker.
#[derive(Clone)]
pub struct AppEngine {
    engine: wasmtime::Engine,
    /// Stops the ticker thread when the last clone goes away.
    _ticker: Arc<Ticker>,
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl AppEngine {
    pub fn new(target: Target) -> Result<AppEngine, EngineError> {
        let engine = wasmtime::Engine::new(&config(target)?).map_err(wt)?;
        let ticker = Arc::new(Ticker {
            stop: AtomicBool::new(false),
        });
        if target != Target::Phone || cfg!(target_pointer_width = "32") {
            let weak = engine.weak();
            let t = Arc::downgrade(&ticker);
            std::thread::Builder::new()
                .name("app-epoch".into())
                .stack_size(32 << 10)
                .spawn(move || {
                    loop {
                        std::thread::sleep(EPOCH_TICK);
                        match (weak.upgrade(), t.upgrade()) {
                            (Some(e), Some(t)) if !t.stop.load(Ordering::Relaxed) => {
                                e.increment_epoch()
                            }
                            _ => break,
                        }
                    }
                })
                .map_err(EngineError::Io)?;
        }
        Ok(AppEngine {
            engine,
            _ticker: ticker,
        })
    }

    pub fn wasmtime(&self) -> &wasmtime::Engine {
        &self.engine
    }

    /// True when this build can compile `.wasm` components.
    pub const fn can_compile() -> bool {
        cfg!(feature = "compile")
    }

    /// Compiles a component to this engine's serialized form (`.cwasm`).
    pub fn precompile(&self, wasm: &[u8]) -> Result<Vec<u8>, EngineError> {
        #[cfg(feature = "compile")]
        {
            self.engine.precompile_component(wasm).map_err(wt)
        }
        #[cfg(not(feature = "compile"))]
        {
            let _ = wasm;
            Err(EngineError::NoCompiler)
        }
    }

    /// Loads serialized code produced by [`AppEngine::precompile`] with the
    /// same Wasmtime version and settings.
    ///
    /// # Safety
    ///
    /// The file must be one this firmware's build (or this phone) produced:
    /// Wasmtime does not validate serialized code against hostile input.
    /// Never call this on a file that came out of an app package.
    pub unsafe fn load_precompiled(&self, path: &Path) -> Result<Component, EngineError> {
        // SAFETY: forwarded to the caller.
        unsafe { Component::deserialize_file(&self.engine, path) }.map_err(wt)
    }

    /// Compiles and loads a `.wasm` component (needs the `compile` feature).
    pub fn load_wasm(&self, wasm: &[u8]) -> Result<Component, EngineError> {
        #[cfg(feature = "compile")]
        {
            Component::from_binary(&self.engine, wasm).map_err(wt)
        }
        #[cfg(not(feature = "compile"))]
        {
            let _ = wasm;
            Err(EngineError::NoCompiler)
        }
    }
}
