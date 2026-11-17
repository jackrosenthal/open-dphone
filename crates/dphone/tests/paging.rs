//! Multicast paging end to end: the real binary in simulator mode, in a
//! private network namespace whose only interface is the loopback (so no
//! multicast leaves the machine).
//!
//! Needs unprivileged user and network namespaces and the `ip` tool; where
//! those are missing it prints a notice and passes without running.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_dphone");
const LISTEN: (Ipv4Addr, u16) = (Ipv4Addr::new(239, 9, 9, 1), 5000);
const PAGE_TO: (Ipv4Addr, u16) = (Ipv4Addr::new(239, 9, 9, 2), 5002);

fn enter_namespace() -> bool {
    // SAFETY: plain system calls with no pointers.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: the process is single-threaded here.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } != 0 {
        return false;
    }
    let mapped = std::fs::write("/proc/self/setgroups", "deny").is_ok()
        && std::fs::write("/proc/self/uid_map", format!("0 {uid} 1")).is_ok()
        && std::fs::write("/proc/self/gid_map", format!("0 {gid} 1")).is_ok();
    let ip = |args: &[&str]| Command::new("ip").args(args).status().is_ok_and(|s| s.success());
    mapped && ip(&["link", "set", "lo", "up"]) && ip(&["route", "add", "224.0.0.0/4", "dev", "lo"])
}

fn diag(socket: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(BIN).args(args).env("DPHONE_DIAG_SOCKET", socket).output().unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// One 20 ms PCMU packet of silence.
fn rtp(seq: u16, ssrc: u32) -> Vec<u8> {
    let mut p = vec![0x80, 0];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&(seq as u32 * 160).to_be_bytes());
    p.extend_from_slice(&ssrc.to_be_bytes());
    p.extend_from_slice(&[0xff; 160]);
    p
}

fn main() {
    if !enter_namespace() {
        println!("paging: SKIPPED (no unprivileged namespaces, or no ip tool)");
        return;
    }
    let dir = tempfile::Builder::new().prefix("dp").tempdir_in("/tmp").unwrap();
    let nv = dir.path().join("nv");
    let run = dir.path().join("run");
    std::fs::create_dir_all(&nv).unwrap();
    std::fs::write(
        nv.join("fetched-config.xml"),
        format!(
            r#"<?xml version="1.0"?>
<config>
  <multicastpage>
    <listener id="1" name="Everyone" addr="{}" port="{}" priority="1" interrupt_calls="1" />
    <broadcast id="dock" name="Loading dock" addr="{}" port="{}" codec="PCMA" />
  </multicastpage>
</config>
"#,
            LISTEN.0, LISTEN.1, PAGE_TO.0, PAGE_TO.1
        ),
    )
    .unwrap();
    let mut main = Command::new(BIN)
        .args(["main", "--sim", "--model", "d60", "--no-ssh", "--nvdata"])
        .arg(&nv)
        .arg("--run-dir")
        .arg(&run)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = run.join("diag.sock");
    let result = std::panic::catch_unwind(|| scenario(&socket));
    let _ = main.kill();
    let _ = main.wait();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
    println!("paging: ok");
}

fn scenario(socket: &Path) {
    wait_for("the page command", || diag(socket, &["page"]).0 == 0);
    wait_for("the groups", || diag(socket, &["page"]).1.contains("listens to 1 'Everyone' on 239.9.9.1:5000"));
    let (_, text) = diag(socket, &["page"]);
    assert!(text.contains("can page dock 'Loading dock' on 239.9.9.2:5002"), "{text}");
    assert!(text.contains("activity: Idle"), "{text}");

    // A page arrives: RTP to the group.  Control datagrams on the same
    // group are not a page.
    let tx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    tx.set_multicast_loop_v4(true).unwrap();
    let group = SocketAddr::from(LISTEN);
    tx.send_to(&[0xd1, 0x61, 0x55, 0x4d, b'p', 0, 0, 0, 0, 0, 0, 0, 0], group).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(diag(socket, &["page"]).1.contains("activity: Idle"));
    let mut seq = 0u16;
    let mut send_for = |ms: u64, ssrc: u32| {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            tx.send_to(&rtp(seq, ssrc), group).unwrap();
            seq = seq.wrapping_add(1);
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    send_for(600, 0x1111);
    let (_, text) = diag(socket, &["page"]);
    assert!(text.contains(r#"activity: Receiving { group: "1", name: "Everyone" }"#), "{text}");
    // The stream reached the audio engine: the state store shows the path
    // open on the loudspeaker.
    let (_, audio) = diag(socket, &["state", "audio"]);
    assert!(audio.contains("path: Speaker"), "{audio}");

    // The source stops; a second later the page is over.
    wait_for("the end of the page", || diag(socket, &["page"]).1.contains("activity: Idle"));
    wait_for("the audio path to close", || diag(socket, &["state", "audio"]).1.contains("path: None"));

    // A page the user dismisses stays dismissed while its source goes on.
    send_for(400, 0x2222);
    assert!(diag(socket, &["page"]).1.contains("Receiving"));
    assert!(diag(socket, &["page", "stop"]).1.contains("activity: Idle"));
    send_for(400, 0x2222);
    assert!(diag(socket, &["page"]).1.contains("activity: Idle"));

    // Paging a group: RTP with the configured codec (PCMA, payload type 8)
    // arrives at the group.
    let rx = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PAGE_TO.1)).unwrap();
    rx.join_multicast_v4(&PAGE_TO.0, &Ipv4Addr::LOCALHOST).unwrap();
    rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let (code, text) = diag(socket, &["page", "start", "dock"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains(r#"activity: Sending { group: "dock", name: "Loading dock" }"#), "{text}");
    let mut buf = [0u8; 1500];
    let (n, _) = rx.recv_from(&mut buf).expect("RTP on the page group");
    assert_eq!((buf[0] >> 6, buf[1] & 0x7f, n), (2, 8, 12 + 160), "an RTP packet of 20 ms of PCMA");
    // While paging, an incoming page is dropped.
    send_for(300, 0x3333);
    assert!(diag(socket, &["page"]).1.contains("Sending"));
    // An unknown group is refused; stopping ends the stream.
    assert_eq!(diag(socket, &["page", "start", "nowhere"]).0, 1);
    assert!(diag(socket, &["page", "stop"]).1.contains("activity: Idle"));
    std::thread::sleep(Duration::from_millis(200));
    // Empty what was received before the stop, then listen again.
    rx.set_nonblocking(true).unwrap();
    while rx.recv_from(&mut buf).is_ok() {}
    std::thread::sleep(Duration::from_millis(300));
    let mut late = 0;
    while rx.recv_from(&mut buf).is_ok() {
        late += 1;
    }
    assert_eq!(late, 0, "the page stream went on after stop");
}
