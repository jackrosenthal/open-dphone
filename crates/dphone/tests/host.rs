//! End-to-end tests on the build host: the real binary, in simulator mode.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_dphone");

/// A temp directory whose paths are short enough for a Unix socket.
fn temp() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("dp").tempdir().unwrap();
    if dir.path().as_os_str().len() < 60 {
        return dir;
    }
    tempfile::Builder::new().prefix("dp").tempdir_in("/tmp").unwrap()
}

fn wait_for(what: &str, timeout: Duration, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The first child process of `parent`, from `/proc`.  (Not `pgrep`: the
/// test must not depend on which tools the machine running it has.)
fn child_of(parent: u32) -> Option<i32> {
    let mut children: Vec<i32> = std::fs::read_dir("/proc")
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            // "PID (COMM) STATE PPID ..."; COMM may itself hold spaces and
            // parentheses, so the fields are counted from its last ')'.
            std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().is_some_and(|stat| {
                stat.rsplit_once(')')
                    .and_then(|(_, rest)| rest.split_ascii_whitespace().nth(1)?.parse::<u32>().ok())
                    == Some(parent)
            })
        })
        .collect();
    children.sort_unstable();
    children.first().copied()
}

/// Prints a file when the test that holds it fails.
struct ShowOnPanic(PathBuf);

impl Drop for ShowOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("--- {} ---\n{}", self.0.display(), std::fs::read_to_string(&self.0).unwrap_or_default());
        }
    }
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn diag(socket: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN).args(args).env("DPHONE_DIAG_SOCKET", socket).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn start_main(dir: &Path, extra: &[&str]) -> (KillOnDrop, PathBuf) {
    let nv = dir.join("nv");
    let run = dir.join("run");
    std::fs::create_dir_all(&nv).unwrap();
    let child = Command::new(BIN)
        .args(["main", "--sim", "--model", "d40", "--nvdata"])
        .arg(&nv)
        .arg("--run-dir")
        .arg(&run)
        .args(extra)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let socket = run.join("diag.sock");
    (KillOnDrop(child), socket)
}

#[test]
fn diag_commands_and_restart() {
    let dir = temp();
    let (mut main, socket) = start_main(dir.path(), &["--no-ssh"]);
    wait_for("diag socket", Duration::from_secs(10), || socket.exists());

    let (code, out, _) = diag(&socket, &["ping"]);
    assert_eq!((code, out.as_str()), (0, "pong\n"));

    let (code, out, _) = diag(&socket, &["version"]);
    assert_eq!(code, 0);
    assert!(out.contains("Open DPhone") && out.contains("model D40"), "{out}");

    let (code, out, _) = diag(&socket, &["help"]);
    assert_eq!(code, 0);
    for name in ["log", "state", "config", "hal", "reboot", "health"] {
        assert!(out.contains(name), "help lacks {name}: {out}");
    }

    // The subsystems wired into the main role answer.
    let (code, out, _) = diag(&socket, &["state", "system"]);
    assert_eq!(code, 0);
    assert!(out.contains("D40") && out.contains("simulated: true"), "{out}");
    let (code, out, _) = diag(&socket, &["config", "get", "network_enable_dhcp"]);
    assert_eq!(code, 0, "{out}");
    let (code, out, _) = diag(&socket, &["hal", "status"]);
    assert_eq!(code, 0);
    assert!(out.contains("simulator") && out.contains("240x120"), "{out}");
    let (code, out, _) = diag(&socket, &["audio"]);
    assert_eq!(code, 0);
    assert!(out.contains("engine: running"), "{out}");
    let (code, out, _) = diag(&socket, &["app", "list"]);
    assert_eq!(code, 0, "{out}");
    let (code, out, _) = diag(&socket, &["net"]);
    assert_eq!(code, 0);
    assert!(out.contains("simulator"), "{out}");

    let (code, _, err) = diag(&socket, &["no-such-command"]);
    assert_eq!(code, 127);
    assert!(err.contains("unknown command"), "{err}");

    wait_for("healthy", Duration::from_secs(10), || diag(&socket, &["health"]).0 == 0);
    let (code, out, _) = diag(&socket, &["log", "-n", "50"]);
    assert_eq!(code, 0);
    assert!(out.contains("main role starting"), "{out}");

    let (code, _, _) = diag(&socket, &["restart"]);
    assert_eq!(code, 0);
    let status = main.0.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// Run the binary with standard output and standard error both connected to
/// pipes that nobody reads, as in `dphone ... 2>&1 | head` after `head` has
/// gone.  Returns the exit status.
fn run_into_closed_pipes(socket: Option<&Path>, args: &[&str]) -> i32 {
    let closed = || {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        Stdio::from(writer)
    };
    let mut cmd = Command::new(BIN);
    cmd.args(args).stdin(Stdio::null()).stdout(closed()).stderr(closed());
    if let Some(socket) = socket {
        cmd.env("DPHONE_DIAG_SOCKET", socket);
    }
    // A panic exits with 101; death by a signal has no exit code.
    cmd.status().unwrap().code().expect("killed by a signal")
}

#[test]
fn output_into_a_closed_pipe_is_not_a_panic() {
    // Commands that run locally: standard output, then standard error.
    assert_eq!(run_into_closed_pipes(None, &["--help"]), 0);
    assert_eq!(run_into_closed_pipes(None, &["--version"]), 0);
    assert_eq!(run_into_closed_pipes(None, &["main", "--help"]), 0);
    assert_eq!(run_into_closed_pipes(None, &["mount"]), 0);
    assert_eq!(run_into_closed_pipes(None, &[]), 2);
    assert_eq!(run_into_closed_pipes(None, &["update"]), 2);
    assert_eq!(run_into_closed_pipes(None, &["update", "switch-bank", "c"]), 1);
    assert_eq!(run_into_closed_pipes(None, &["umount"]), 1);
    assert_eq!(run_into_closed_pipes(None, &["main", "--no-such-option"]), 2);
    assert_eq!(run_into_closed_pipes(None, &["init", "--no-such-option"]), 2);

    // Commands relayed from the main role, and the main role not being there.
    let dir = temp();
    let (_main, socket) = start_main(dir.path(), &["--no-ssh"]);
    assert_eq!(run_into_closed_pipes(Some(&dir.path().join("none.sock")), &["ping"]), 1);
    wait_for("diag socket", Duration::from_secs(10), || socket.exists());
    wait_for("main role", Duration::from_secs(10), || diag(&socket, &["ping"]).0 == 0);
    assert_eq!(run_into_closed_pipes(Some(&socket), &["help"]), 1);
    assert_eq!(run_into_closed_pipes(Some(&socket), &["no-such-command"]), 127);
    // Following the log would never end by itself.
    assert_eq!(run_into_closed_pipes(Some(&socket), &["log", "-f"]), 1);
}

#[test]
fn supervisor_confirms_trial_and_restarts_main() {
    let dir = temp();
    let nv = dir.path().join("nv");
    let run = dir.path().join("run");
    std::fs::create_dir_all(nv.join("open")).unwrap();
    std::fs::create_dir_all(&run).unwrap();
    let trial = nv.join("open/trial");
    std::fs::write(&trial, "pending=1\nprevious_bank=0\nboots=0\nmax_boots=3\n").unwrap();

    // What the supervisor and the main roles log, shown if the test fails.
    let log = dir.path().join("log");
    let _show_log = ShowOnPanic(log.clone());
    let sup = Command::new(BIN)
        .args(["init", "--supervise-only", "--nvdata"])
        .arg(&nv)
        .arg("--run-dir")
        .arg(&run)
        .args(["--grace", "20", "--beat-timeout", "3", "--"])
        .arg(BIN)
        .args(["main", "--sim", "--no-ssh", "--nvdata"])
        .arg(&nv)
        .arg("--run-dir")
        .arg(&run)
        .stdin(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let sup = KillOnDrop(sup);
    let socket = run.join("diag.sock");

    // The main role reports healthy; the trial is confirmed.
    wait_for("trial confirmation", Duration::from_secs(15), || !trial.exists());

    let main_pid = |sup: &KillOnDrop| child_of(sup.0.id());
    wait_for("main role", Duration::from_secs(10), || diag(&socket, &["ping"]).0 == 0);
    let first = main_pid(&sup).expect("main role pid");

    // Kill it: the supervisor starts another.
    // SAFETY: plain kill(2) on a child of our child.
    unsafe { libc::kill(first, libc::SIGKILL) };
    wait_for("restart after kill", Duration::from_secs(15), || {
        main_pid(&sup).is_some_and(|p| p != first) && diag(&socket, &["ping"]).0 == 0
    });
    let second = main_pid(&sup).unwrap();

    // Hang it: the heartbeat stops, the supervisor kills and restarts it.
    // The short beat timeout applies only once the supervisor has had a beat
    // from this instance; one that has not beaten yet is given the whole
    // start grace.  The diagnostics socket answers before the first beat is
    // written, so "answers ping" is too early to stop it: wait until the
    // supervisor says it has heard from this instance (its second such line;
    // the first was for the instance that confirmed the trial).
    let heard = || std::fs::read_to_string(&log).unwrap_or_default().matches("main role reports healthy").count();
    wait_for("second instance healthy", Duration::from_secs(15), || heard() >= 2);
    // SAFETY: as above.
    unsafe { libc::kill(second, libc::SIGSTOP) };
    wait_for("restart after hang", Duration::from_secs(20), || {
        main_pid(&sup).is_some_and(|p| p != second) && diag(&socket, &["ping"]).0 == 0
    });

    // SIGTERM to the supervisor ("reboot"): it stops the main role and ends.
    let third = main_pid(&sup).unwrap();
    // SAFETY: as above.
    unsafe { libc::kill(sup.0.id() as i32, libc::SIGTERM) };
    wait_for("main role stopped", Duration::from_secs(10), || unsafe { libc::kill(third, 0) } != 0);
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("-V").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

#[test]
fn ssh_exec_and_sftp_with_openssh_client() {
    if !have("ssh") || !have("scp") || !Path::new("/bin/sh").exists() {
        // The Nix check provides the client and sets this, so that the test
        // cannot pass there by not running.
        assert!(std::env::var_os("DPHONE_TEST_REQUIRE_SSH").is_none(), "no OpenSSH client (ssh, scp) or no /bin/sh");
        eprintln!("skipped: no OpenSSH client on this machine");
        return;
    }
    let dir = temp();
    let d = dir.path();
    let key = d.join("id");
    let bad = d.join("bad");
    for k in [&key, &bad] {
        let ok = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(k).status().unwrap();
        assert!(ok.success());
    }
    // An RSA key too: that is what the stock phones' authorized_keys hold.
    let rsa = d.join("id_rsa");
    let ok = Command::new("ssh-keygen").args(["-q", "-t", "rsa", "-b", "3072", "-N", "", "-f"]).arg(&rsa).status().unwrap();
    assert!(ok.success());
    // No key file yet: the server starts with nobody able to log in.
    std::fs::create_dir_all(d.join("nv/root/.ssh")).unwrap();
    std::fs::create_dir_all(d.join("nv/open")).unwrap();

    let nv = d.join("nv");
    let run = d.join("run");
    let mut child = Command::new(BIN)
        .args(["main", "--sim", "--ssh-listen", "127.0.0.1:0", "--nvdata"])
        .arg(&nv)
        .arg("--run-dir")
        .arg(&run)
        .env("DPHONE_SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let _main = KillOnDrop(child);
    let mut port = None;
    for line in BufReader::new(stderr).lines() {
        let line = line.unwrap();
        if let Some(rest) = line.split("ssh listening on 127.0.0.1:").nth(1) {
            port = rest.trim().parse::<u16>().ok();
            break;
        }
    }
    let port = port.expect("ssh port in log").to_string();

    let known = d.join("known_hosts");
    let options = |cmd: &mut Command, identity: &Path| {
        cmd.args(["-o", "StrictHostKeyChecking=no", "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes"])
            .args(["-o", "IdentityAgent=none", "-o", "AddKeysToAgent=no", "-o", "ControlMaster=no"])
            .args(["-o", "ControlPath=none", "-o", "LogLevel=ERROR", "-F", "/dev/null"])
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", known.display()))
            .arg("-i")
            .arg(identity)
            .stdin(Stdio::null());
    };
    let ssh = |identity: &Path, user: &str, command: &str| {
        let mut cmd = Command::new("ssh");
        options(&mut cmd, identity);
        cmd.args(["-p", &port]).arg(format!("{user}@127.0.0.1")).arg(command);
        cmd.output().unwrap()
    };

    // No key is compiled in, so with no key file nobody gets in.
    assert_eq!(ssh(&key, "root", "true").status.code(), Some(255));
    assert_eq!(ssh(&rsa, "root", "true").status.code(), Some(255));

    // Keys are read at each attempt, from both files: the user's own, and
    // the provisioned ones.  No restart in between.
    std::fs::copy(d.join("id.pub"), d.join("nv/root/.ssh/authorized_keys")).unwrap();
    assert_eq!(ssh(&key, "root", "true").status.code(), Some(0));
    assert_eq!(ssh(&rsa, "root", "true").status.code(), Some(255));
    std::fs::copy(d.join("id_rsa.pub"), d.join("nv/open/provisioned_authorized_keys")).unwrap();

    // Command, output on both streams, exit status.
    let out = ssh(&key, "root", "echo out; echo err >&2; exit 7");
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "out\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("err"));

    // RSA public-key login.
    let out = ssh(&rsa, "root", "echo rsa");
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).as_ref()), (Some(0), "rsa\n"), "{out:?}");

    // Unknown key and unknown user are refused.
    assert_eq!(ssh(&bad, "root", "true").status.code(), Some(255));
    assert_eq!(ssh(&key, "admin", "true").status.code(), Some(255));

    // A terminal session.
    let mut cmd = Command::new("ssh");
    options(&mut cmd, &key);
    let out = cmd.args(["-tt", "-p", &port, "root@127.0.0.1", "tty; exit 3"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stdout).contains("/dev/pts/"), "{:?}", out);

    // Push a file with scp (SFTP underneath) and read it back over exec.
    {
        let payload: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let local = d.join("payload.bin");
        let remote = d.join("pushed.bin");
        std::fs::write(&local, &payload).unwrap();
        let mut cmd = Command::new("scp");
        options(&mut cmd, &key);
        let out = cmd.args(["-P", &port]).arg(&local).arg(format!("root@127.0.0.1:{}", remote.display())).output().unwrap();
        assert!(out.status.success(), "scp: {:?}", out);
        assert_eq!(std::fs::read(&remote).unwrap(), payload);
        let out = ssh(&key, "root", &format!("cat {}", remote.display()));
        assert_eq!(out.stdout, payload);

        // Into a directory, as in `scp file root@phone:/tmp/`, keeping the
        // mode (-p also sends times, which the server accepts).
        let into = d.join("into");
        std::fs::create_dir(&into).unwrap();
        std::fs::set_permissions(&local, std::os::unix::fs::PermissionsExt::from_mode(0o750)).unwrap();
        let mut cmd = Command::new("scp");
        options(&mut cmd, &key);
        let out = cmd.args(["-p", "-P", &port]).arg(&local).arg(format!("root@127.0.0.1:{}/", into.display())).output().unwrap();
        assert!(out.status.success(), "scp into a directory: {:?}", out);
        let pushed = into.join("payload.bin");
        assert_eq!(std::fs::read(&pushed).unwrap(), payload);
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&pushed).unwrap().permissions()) & 0o777, 0o750);

        // And back.
        let fetched = d.join("fetched.bin");
        let mut cmd = Command::new("scp");
        options(&mut cmd, &key);
        let out = cmd.args(["-P", &port]).arg(format!("root@127.0.0.1:{}", pushed.display())).arg(&fetched).output().unwrap();
        assert!(out.status.success(), "scp from the phone: {:?}", out);
        assert_eq!(std::fs::read(&fetched).unwrap(), payload);

        // A directory tree.
        let mut cmd = Command::new("scp");
        options(&mut cmd, &key);
        let out = cmd.args(["-r", "-P", &port]).arg(&into).arg(format!("root@127.0.0.1:{}", d.join("tree").display())).output().unwrap();
        assert!(out.status.success(), "scp -r: {:?}", out);
        assert_eq!(std::fs::read(d.join("tree/payload.bin")).unwrap(), payload);
    }
}
