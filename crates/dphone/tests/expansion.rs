//! An expansion module end to end: the real binary in simulator mode and a
//! scripted module, in a private network namespace (loopback only).
//!
//! Needs unprivileged user and network namespaces and the `ip` tool; where
//! those are missing it prints a notice and passes without running.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_dphone");
const PHONE: Ipv4Addr = Ipv4Addr::LOCALHOST;
const MODULE: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);

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
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn main() {
    if !enter_namespace() {
        println!("expansion: SKIPPED (no unprivileged namespaces, or no ip tool)");
        return;
    }
    let dir = tempfile::Builder::new().prefix("dp").tempdir_in("/tmp").unwrap();
    let nv = dir.path().join("nv");
    let run = dir.path().join("run");
    std::fs::create_dir_all(&nv).unwrap();
    std::fs::write(
        nv.join("fetched-config.xml"),
        r#"<?xml version="1.0"?>
<config>
  <setting id="expansion_enable" value="1" type="EXP100" />
  <smart_blf>
    <blf_items>
      <blf_item location="expansion" node="0" index="3" app_id="voicemail" />
      <blf_item location="expansion" node="0" index="25" app_id="parking" />
    </blf_items>
  </smart_blf>
</config>
"#,
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
    println!("expansion: ok");
}

/// A UDP socket on `port` that shares the port with the phone's.
fn reuse_bind(port: u16) -> UdpSocket {
    use std::os::fd::FromRawFd;
    // SAFETY: plain socket calls on a descriptor owned by the returned
    // socket; pointers are to live locals of the stated size.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        assert!(fd >= 0);
        let one: libc::c_int = 1;
        assert_eq!(libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, (&raw const one).cast(), 4), 0);
        let addr = libc::sockaddr_in { sin_family: libc::AF_INET as libc::sa_family_t, sin_port: port.to_be(), sin_addr: libc::in_addr { s_addr: 0 }, sin_zero: [0; 8] };
        assert_eq!(libc::bind(fd, (&raw const addr).cast(), size_of::<libc::sockaddr_in>() as libc::socklen_t), 0);
        UdpSocket::from_raw_fd(fd)
    }
}

fn text(socket: &UdpSocket) -> String {
    let mut buf = [0u8; 8192];
    let (n, _) = socket.recv_from(&mut buf).expect("a datagram from the phone");
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

fn scenario(socket: &Path) {
    wait_for("the subsystem", || diag(socket, &["expansion"]).1.contains("enabled: true"));
    // The module: its own address and port, and an ear on the announce group.
    let module = UdpSocket::bind((MODULE, 10025)).unwrap();
    module.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    module.set_multicast_loop_v4(true).unwrap();
    let announce = reuse_bind(10026);
    announce.join_multicast_v4(&Ipv4Addr::new(224, 0, 3, 1), &PHONE).unwrap();
    announce.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let phone = SocketAddr::from((PHONE, 10025));

    // Discovery, repeated until the phone has its sockets open.
    let mut ack = None;
    wait_for("the discovery answer", || {
        module.send_to(b"EXP_NOTIFY\nVersion:v5.0\n", (Ipv4Addr::new(224, 0, 2, 1), 10024)).unwrap();
        let mut buf = [0u8; 256];
        match announce.recv_from(&mut buf) {
            Ok((n, _)) => {
                ack = Some(String::from_utf8_lossy(&buf[..n]).into_owned());
                true
            }
            Err(_) => false,
        }
    });
    assert_eq!(ack.as_deref(), Some("EXP_NOTIFY_ACK\nEXP_NO:0"));
    assert!(diag(socket, &["expansion"]).1.contains("module 0 at 127.0.0.2 version 'v5.0' page 1"));

    // Liveness and status questions.
    module.send_to(b"SOCKET_DETECT_EVENT\nEXP_NO:0\n", phone).unwrap();
    assert_eq!(text(&module), "SOCKET_DETECT_EVENT_ACK\nEXP_NO:0\n");
    module.send_to(b"EXP_SYN_ACK\n", phone).unwrap();
    assert_eq!(text(&module), "EXP_SYN_ACK_OK\nEXP_NO:0\n");
    module.send_to(b"DND_GET_EVENT\n", phone).unwrap();
    assert_eq!(text(&module), "EXP_DND_STATUS\nDND_FLAG:0\n");
    module.send_to(b"CHECK_VERSION_EVENT\n", phone).unwrap();
    assert_eq!(text(&module), "CHECK_VERSION_EVENT_ACK\nHAS_VERSION:0\nEXP_NO:0\n");

    // The key configuration: the two configured keys, on their pages.
    module.send_to(b"CFG_GET_EVENT\n", phone).unwrap();
    let config = text(&module);
    assert!(config.starts_with("EXP_CFG_GET_ACK\nEXP_CFG_SAVE_EVENT\nEVENT_ID:{"), "{config}");
    assert!(config.contains("<KEY_3>\nKeyNo:0\nDssType:3\nValue:\naid:0\nPickUpCode:**\nLabel:voicemail\n"), "{config}");
    let second_page = config.split("<EXP_CFG_PAGE_2>\n").nth(1).expect("a second page");
    assert!(second_page.contains("<KEY_5>\nKeyNo:0\nDssType:3\nValue:\naid:0\nPickUpCode:**\nLabel:parking\n"));
    assert!(config.ends_with("<END>"));

    // Stored: the phone sets the title.  Then a page change renames it.
    module.send_to(b"EXP_CFG_SAVE_EVENT_ACK\n", phone).unwrap();
    assert!(text(&module).starts_with("EXP_SET_TITLE\nTITLE_TEXT:Page 1/2\n"));
    module.send_to(b"EXP_PAGE_EVENT\nEVENT_ID:{5}\nEXP_PAGE:1\nEXP_PAGE_KEY:0\n", phone).unwrap();
    assert!(text(&module).starts_with("EXP_SET_TITLE\nTITLE_TEXT:Page 2/2\n"));
    assert!(diag(socket, &["expansion"]).1.contains("page 2"));

    // A key press is acknowledged the way stock does it.
    module.send_to(b"EXP_KEYEVENT\nEVENT_ID:{9}\nEXP_NO:0\nEXP_PAGE:0\nEXP_KEYNO:3\n", phone).unwrap();
    assert_eq!(text(&module), "EXP_KEYEVENT_ACK\nEVENT_ID:{9}EXP_KEYNO:3");
}
