//! The EHS subsystem and the UART hand-over, on the build host: the real
//! binary in simulator mode.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_dphone");

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("dp").tempdir().unwrap();
    if dir.path().as_os_str().len() < 60 {
        return dir;
    }
    tempfile::Builder::new().prefix("dp").tempdir_in("/tmp").unwrap()
}

fn diag(socket: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(BIN).args(args).env("DPHONE_DIAG_SOCKET", socket).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn start(dir: &Path, settings: Option<&str>) -> (KillOnDrop, std::path::PathBuf) {
    let nv = dir.join("nv");
    let run = dir.join("run");
    std::fs::create_dir_all(nv.join("opendeskphone")).unwrap();
    if let Some(text) = settings {
        std::fs::write(nv.join("opendeskphone/settings.txtpb"), text).unwrap();
    }
    let child = Command::new(BIN)
        .args(["main", "--sim", "--model", "d60", "--no-ssh", "--nvdata"])
        .arg(&nv)
        .arg("--run-dir")
        .arg(&run)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = run.join("diag.sock");
    wait_for("the ehs command", || diag(&socket, &["ehs"]).0 == 0);
    (KillOnDrop(child), run)
}

#[test]
fn ehs_has_the_uart_by_default() {
    let dir = temp();
    let (_main, run) = start(dir.path(), None);
    let socket = run.join("diag.sock");
    wait_for("the adapter search", || diag(&socket, &["ehs"]).1.contains("uart: claimed"));
    let (_, text) = diag(&socket, &["ehs"]);
    // The stock default for the "ehs" setting is auto-detection.
    assert!(text.contains("mode: Auto") && text.contains("adapter: none detected"), "{text}");
    // Nothing tells the init role to run a console shell.
    assert!(!run.join("serial-console").exists());
}

#[test]
fn the_console_gets_the_uart_when_switched_on() {
    let dir = temp();
    let (_main, run) = start(dir.path(), Some("serial_console: true\n"));
    let socket = run.join("diag.sock");
    wait_for("the hand-over", || run.join("serial-console").exists());
    let (_, text) = diag(&socket, &["ehs"]);
    assert!(text.starts_with("off:"), "{text}");
}

#[test]
fn ehs_takes_the_uart_back_from_a_console_the_init_role_granted() {
    let dir = temp();
    // What the init role leaves after a main role failed.
    std::fs::create_dir_all(dir.path().join("run")).unwrap();
    std::fs::write(dir.path().join("run/serial-console"), "init\n").unwrap();
    let (_main, run) = start(dir.path(), Some("serial_console: false\n"));
    wait_for("the file to go", || !run.join("serial-console").exists());
    wait_for("the adapter search", || diag(&run.join("diag.sock"), &["ehs"]).1.contains("uart: claimed"));
}
