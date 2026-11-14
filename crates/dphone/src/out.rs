//! Output of the command-line roles to standard output and standard error.
//!
//! Rust starts every program with `SIGPIPE` ignored, so writing to a pipe
//! whose reader has gone (`dphone update status | head`) returns an error,
//! and `print!` and `eprint!` turn that error into a panic.  Everything this
//! binary prints for a person therefore goes through [`out!`], [`outln!`],
//! [`err!`], and [`errln!`], which do not.  (The crate denies the standard
//! macros, so that a new output path cannot bring the panic back.)
//!
//! `SIGPIPE` is deliberately left ignored rather than restored: a command
//! that is writing flash (`dphone update FILE`) must run to its end whether
//! or not anybody is still reading its progress.  Once standard output has
//! failed, later output to it is dropped without further attempts, and the
//! command carries on; a command that would otherwise never end (e.g.,
//! `dphone log -f`) asks [`stdout_gone`] and stops.

use std::fmt::Arguments;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static STDOUT_GONE: AtomicBool = AtomicBool::new(false);
static STDERR_GONE: AtomicBool = AtomicBool::new(false);

fn emit(gone: &AtomicBool, write: impl FnOnce() -> std::io::Result<()>) {
    if !gone.load(Ordering::Relaxed) && write().is_err() {
        gone.store(true, Ordering::Relaxed);
    }
}

/// Write formatted text to standard output; see [`out!`].
pub fn stdout(args: Arguments<'_>) {
    emit(&STDOUT_GONE, || std::io::stdout().write_fmt(args));
}

/// Write bytes to standard output and flush them.
pub fn stdout_bytes(data: &[u8]) {
    emit(&STDOUT_GONE, || {
        let mut out = std::io::stdout().lock();
        out.write_all(data)?;
        out.flush()
    });
}

/// Flush standard output (it is line-buffered; a progress line that ends in
/// no newline needs this).
pub fn flush() {
    emit(&STDOUT_GONE, || std::io::stdout().flush());
}

/// Whether a write to standard output has failed (its reader is gone).
pub fn stdout_gone() -> bool {
    STDOUT_GONE.load(Ordering::Relaxed)
}

/// Write formatted text to standard error; see [`err!`].
pub fn stderr(args: Arguments<'_>) {
    emit(&STDERR_GONE, || std::io::stderr().write_fmt(args));
}

/// Write bytes to standard error.
pub fn stderr_bytes(data: &[u8]) {
    emit(&STDERR_GONE, || std::io::stderr().write_all(data));
}

/// `print!` that survives a closed pipe.
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => { $crate::out::stdout(::std::format_args!($($arg)*)) };
}

/// `println!` that survives a closed pipe.
#[macro_export]
macro_rules! outln {
    () => { $crate::out::stdout(::std::format_args!("\n")) };
    ($($arg:tt)*) => { $crate::out::stdout(::std::format_args!("{}\n", ::std::format_args!($($arg)*))) };
}

/// `eprint!` that survives a closed pipe.
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => { $crate::out::stderr(::std::format_args!($($arg)*)) };
}

/// `eprintln!` that survives a closed pipe.
#[macro_export]
macro_rules! errln {
    ($($arg:tt)*) => { $crate::out::stderr(::std::format_args!("{}\n", ::std::format_args!($($arg)*))) };
}
