//! Open Desk Phone: the single userspace binary.  See `dphone::cli` for
//! dispatch.

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    std::process::exit(dphone::cli::run(args));
}
