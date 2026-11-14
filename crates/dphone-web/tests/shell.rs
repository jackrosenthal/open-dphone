//! End-to-end tests of the web terminal, the SSH keys and switch, and the
//! provisioned-firmware policy: the real server on a loopback port, driven
//! over raw HTTP/1.1.

use buffa::Message;
use dphone_config::{ConfigStore, Nvdata};
use dphone_state::{Model, Store};
use dphone_web::{pb, PtyShellHost, SshControl, WebOptions, WebServer};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

type Event = pb::__buffa::oneof::open_shell_response::Event;

// Throwaway keys made with ssh-keygen; the fingerprints are what
// `ssh-keygen -l -f` printed.
const ED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINxjRiA9rj/Ivh47IO5Y/LiMYa5eF7rf3mEK6rzozUOj ed@test";
const ED_FP: &str = "SHA256:ChrmgdKKoWT6FLWac+x9RMS4LL11uAOZpq33PI1lmj4";
const EC256: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBGFiy0K6rIDTVhBd2p4Yar54GvVgqQvPdHdH/oO62UPlWZbIj2fia21ebA/ZQ2RLqkNJAZ730vLQ/KHDMJh6HN4= ec256";
const EC256_FP: &str = "SHA256:VMNvUiMbWDOWfOuU2Ck8livrtYwwmciKtXXs+NJ2V00";

struct Phone {
    addr: SocketAddr,
    config: Arc<Mutex<ConfigStore>>,
    dir: PathBuf,
}

impl Drop for Phone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An [`SshControl`] that records what it was told, as the integrator's
/// would store the setting and switch the listener.
struct FakeSsh {
    config: Arc<Mutex<ConfigStore>>,
    listening: AtomicBool,
}

impl SshControl for FakeSsh {
    fn set_enabled(&self, enabled: bool) -> std::io::Result<()> {
        self.config.lock().unwrap().update_dphone_settings(|s| s.ssh_enabled = Some(enabled)).map_err(std::io::Error::other)?;
        self.listening.store(enabled, Ordering::SeqCst);
        Ok(())
    }
    fn listening(&self) -> Option<bool> {
        Some(self.listening.load(Ordering::SeqCst))
    }
}

async fn phone(with_control: bool) -> Phone {
    let mut id = [0u8; 8];
    getrandom::fill(&mut id).unwrap();
    let dir = std::env::temp_dir().join(format!("dphone-web-shell-{}", id.map(|b| format!("{b:02x}")).concat()));
    std::fs::create_dir_all(dir.join("nvdata")).unwrap();
    let nv = Nvdata::new(dir.join("nvdata"));
    nv.ensure_layout().unwrap();
    let config = Arc::new(Mutex::new(ConfigStore::open(nv, dphone_config::Model::D60, dir.join("tmp")).unwrap()));
    let options = WebOptions {
        http: Some("127.0.0.1:0".parse().unwrap()),
        https: None,
        upload_dir: dir.join("upload"),
        ..Default::default()
    };
    let shell = PtyShellHost::new("/bin/sh").env_clear().env("PATH", "/usr/bin:/bin").env("PS1", "prompt> ").current_dir(&dir);
    let mut server = WebServer::new(Store::new(Model::D60), config.clone(), options).with_shell(Arc::new(shell));
    if with_control {
        server = server.with_ssh_control(Arc::new(FakeSsh { config: config.clone(), listening: AtomicBool::new(true) }));
    }
    let bound = server.bind().await.unwrap();
    let addr = bound.http_addr().unwrap();
    tokio::spawn(bound.serve());
    Phone { addr, config, dir }
}

struct Login {
    cookie: String,
    csrf: String,
}

/// Reads a whole response on a fresh connection: status and body.
async fn post(addr: SocketAddr, path: &str, headers: &[(&str, &str)], body: &[u8]) -> (u16, Vec<u8>, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut head = format!("POST {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), s.read_to_end(&mut raw)).await.expect("response in time").unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("header end");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    let mut body = raw[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let mut out = Vec::new();
        let mut data = &body[..];
        while let Some(eol) = data.windows(2).position(|w| w == b"\r\n") {
            let len = usize::from_str_radix(std::str::from_utf8(&data[..eol]).unwrap().trim(), 16).unwrap();
            if len == 0 {
                break;
            }
            out.extend_from_slice(&data[eol + 2..eol + 2 + len]);
            data = &data[eol + 2 + len + 2..];
        }
        body = out;
    }
    (status, body, head)
}

async fn login(addr: SocketAddr) -> Login {
    let (status, body, head) = post(
        addr,
        "/dphone.v1.AuthService/Login",
        &[("Content-Type", "application/json")],
        br#"{"user":"admin","password":"789"}"#,
    )
    .await;
    assert_eq!(status, 200);
    let cookie = head
        .lines()
        .find_map(|l| l.strip_prefix("set-cookie: ").or_else(|| l.strip_prefix("Set-Cookie: ")))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    Login { cookie, csrf: v["session"]["csrfToken"].as_str().unwrap().to_owned() }
}

/// A unary call with the JSON codec: HTTP status and the JSON body.
async fn call(addr: SocketAddr, l: &Login, method: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
    let (status, body, _) = post(
        addr,
        &format!("/dphone.v1.{method}"),
        &[("Content-Type", "application/json"), ("Cookie", &l.cookie), ("X-Csrf-Token", &l.csrf)],
        body.to_string().as_bytes(),
    )
    .await;
    (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
}

/// An open `OpenShell` stream, read message by message.
struct Terminal {
    stream: TcpStream,
    /// Bytes received and not yet consumed: chunked transfer coding first,
    /// then Connect envelopes.
    wire: Vec<u8>,
    frames: Vec<u8>,
    headers_done: bool,
    id: String,
    /// Everything the shell has printed so far.
    seen: Vec<u8>,
    exited: Option<pb::ShellExited>,
    /// The end-of-stream frame's JSON, once received.
    end: Option<serde_json::Value>,
}

impl Terminal {
    async fn open(addr: SocketAddr, l: &Login, cols: u32, rows: u32) -> Terminal {
        let msg = pb::OpenShellRequest { cols, rows }.encode_to_vec();
        let mut framed = vec![0u8];
        framed.extend((msg.len() as u32).to_be_bytes());
        framed.extend(&msg);
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let head = format!(
            "POST /dphone.v1.ShellService/OpenShell HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/connect+proto\r\nCookie: {}\r\nX-Csrf-Token: {}\r\nContent-Length: {}\r\n\r\n",
            l.cookie,
            l.csrf,
            framed.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(&framed).await.unwrap();
        let mut t = Terminal {
            stream,
            wire: Vec::new(),
            frames: Vec::new(),
            headers_done: false,
            id: String::new(),
            seen: Vec::new(),
            exited: None,
            end: None,
        };
        t.until(|t| !t.id.is_empty() || t.end.is_some()).await;
        t
    }

    /// Moves complete chunks from `wire` to `frames`, and complete
    /// envelopes from `frames` into the terminal's state.
    fn digest(&mut self) {
        if !self.headers_done {
            let Some(split) = self.wire.windows(4).position(|w| w == b"\r\n\r\n") else { return };
            let head = String::from_utf8_lossy(&self.wire[..split]).into_owned();
            assert!(head.starts_with("HTTP/1.1 200"), "{head}");
            self.wire.drain(..split + 4);
            self.headers_done = true;
        }
        while let Some(eol) = self.wire.windows(2).position(|w| w == b"\r\n") {
            let len = usize::from_str_radix(std::str::from_utf8(&self.wire[..eol]).unwrap().trim(), 16).unwrap();
            if self.wire.len() < eol + 2 + len + 2 {
                break;
            }
            self.frames.extend_from_slice(&self.wire[eol + 2..eol + 2 + len]);
            self.wire.drain(..eol + 2 + len + 2);
            if len == 0 {
                break;
            }
        }
        while self.frames.len() >= 5 {
            let len = u32::from_be_bytes(self.frames[1..5].try_into().unwrap()) as usize;
            if self.frames.len() < 5 + len {
                break;
            }
            let flags = self.frames[0];
            let body: Vec<u8> = self.frames[5..5 + len].to_vec();
            self.frames.drain(..5 + len);
            if flags & 2 != 0 {
                self.end = Some(serde_json::from_slice(&body).unwrap());
                continue;
            }
            match pb::OpenShellResponse::decode_from_slice(&body).unwrap().event {
                Some(Event::Started(s)) => self.id = s.session_id.clone(),
                Some(Event::Output(o)) => self.seen.extend_from_slice(&o),
                Some(Event::Exited(e)) => self.exited = Some(*e),
                None => panic!("an empty event"),
            }
        }
    }

    /// Reads until `done` holds; panics after ten seconds.
    async fn until(&mut self, done: impl Fn(&Terminal) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut buf = [0u8; 8192];
        loop {
            self.digest();
            if done(self) {
                return;
            }
            match tokio::time::timeout_at(deadline, self.stream.read(&mut buf)).await {
                Ok(Ok(0)) => panic!("stream closed; saw {:?}", self.text()),
                Ok(Ok(n)) => self.wire.extend_from_slice(&buf[..n]),
                Ok(Err(e)) => panic!("read: {e}"),
                Err(_) => panic!("timed out; saw {:?}", self.text()),
            }
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen).into_owned()
    }

    async fn expect(&mut self, needle: &str) {
        self.until(|t| t.text().contains(needle)).await;
    }
}

async fn write(addr: SocketAddr, l: &Login, id: &str, text: &str) -> (u16, serde_json::Value) {
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(text);
    call(addr, l, "ShellService/WriteShell", serde_json::json!({ "sessionId": id, "data": data })).await
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn wait_gone(pid: i32) {
    for _ in 0..100 {
        if !alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("process {pid} is still running");
}

/// The number a command printed as `<tag>=<number>=` on a line of its own
/// (the typed command's echo has `$` where the number goes).
fn printed_number(text: &str, tag: &str) -> Option<i32> {
    let prefix = format!("{tag}=");
    // The line may begin with terminal escapes (bash turns bracketed paste
    // off before running a command).
    text.lines()
        .map(str::trim)
        .filter(|l| l.ends_with('=') && !l.contains('$'))
        .find_map(|l| l.rfind(&prefix).and_then(|at| l[at + prefix.len()..l.len() - 1].parse().ok()))
}

/// The shell's process id, by asking it.
async fn shell_pid(addr: SocketAddr, l: &Login, t: &mut Terminal) -> i32 {
    assert_eq!(write(addr, l, &t.id, "echo pid=$$=\n").await.0, 200);
    t.until(|t| printed_number(&t.text(), "pid").is_some()).await;
    printed_number(&t.text(), "pid").unwrap()
}

#[tokio::test]
async fn open_write_resize_exit() {
    let p = phone(false).await;
    let l = login(p.addr).await;
    let mut t = Terminal::open(p.addr, &l, 80, 24).await;
    assert_eq!(t.id.len(), 32);
    t.expect("prompt> ").await;

    // Input reaches the shell; its output comes back, with the terminal's
    // line discipline in between (echo, CR LF).
    assert_eq!(write(p.addr, &l, &t.id, "echo he''llo; stty size\n").await.0, 200);
    t.expect("hello\r\n24 80\r\n").await;

    // A resize is seen by programs on the terminal.
    let (status, _) =
        call(p.addr, &l, "ShellService/ResizeShell", serde_json::json!({ "sessionId": t.id, "cols": 100, "rows": 30 })).await;
    assert_eq!(status, 200);
    assert_eq!(write(p.addr, &l, &t.id, "stty size\n").await.0, 200);
    t.expect("30 100\r\n").await;

    // Ctrl-C interrupts the foreground program.
    assert_eq!(write(p.addr, &l, &t.id, "sleep 30; echo not-re''ached\n").await.0, 200);
    t.expect("sleep 30").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(write(p.addr, &l, &t.id, "\u{3}").await.0, 200);
    assert_eq!(write(p.addr, &l, &t.id, "echo af''ter\n").await.0, 200);
    t.expect("after\r\n").await;
    assert!(!t.text().contains("not-reached\r\n"));

    // A paste larger than the terminal's input queue (4 KiB) arrives whole,
    // while the shell echoes it back.
    let body: String = (0..400).map(|i| format!("line {i:03} {}\n", "x".repeat(40))).collect();
    assert!(body.len() > 16 << 10);
    let paste = format!("cat > pasted.txt <<'EOF'\n{body}EOF\n");
    assert_eq!(write(p.addr, &l, &t.id, &paste).await.0, 200);
    assert_eq!(write(p.addr, &l, &t.id, "echo size=$(wc -c < pasted.txt)=\n").await.0, 200);
    t.until(|t| printed_number(&t.text(), "size").is_some()).await;
    assert_eq!(printed_number(&t.text(), "size"), Some(body.len() as i32));
    assert_eq!(std::fs::read_to_string(p.dir.join("pasted.txt")).unwrap(), body);

    // The shell's exit status ends the stream.
    assert_eq!(write(p.addr, &l, &t.id, "exit 3\n").await.0, 200);
    t.until(|t| t.exited.is_some() && t.end.is_some()).await;
    let exited = t.exited.clone().unwrap();
    assert_eq!((exited.exit_code, exited.signal, exited.reason.as_str()), (3, 0, ""));
    assert_eq!(t.end, Some(serde_json::json!({})));

    // The session is gone.
    let (status, v) = write(p.addr, &l, &t.id, "echo\n").await;
    assert_eq!((status, v["code"].as_str()), (404, Some("not_found")));
}

#[tokio::test]
async fn session_cap_ownership_and_cleanup() {
    let p = phone(false).await;
    let l = login(p.addr).await;
    let mut a = Terminal::open(p.addr, &l, 80, 24).await;
    let mut b = Terminal::open(p.addr, &l, 80, 24).await;
    a.expect("prompt> ").await;
    b.expect("prompt> ").await;
    let pid_a = shell_pid(p.addr, &l, &mut a).await;
    let pid_b = shell_pid(p.addr, &l, &mut b).await;
    assert!(alive(pid_a) && alive(pid_b) && pid_a != pid_b);

    // Two shells at most.
    let c = Terminal::open(p.addr, &l, 80, 24).await;
    assert_eq!(c.end.as_ref().unwrap()["error"]["code"], "resource_exhausted");
    assert!(c.id.is_empty());

    // Another login cannot write to, resize, or close these shells.
    let other = login(p.addr).await;
    let (status, v) = write(p.addr, &other, &a.id, "echo intruder\n").await;
    assert_eq!((status, v["code"].as_str()), (404, Some("not_found")));
    let (status, _) = call(p.addr, &other, "ShellService/CloseShell", serde_json::json!({ "sessionId": a.id })).await;
    assert_eq!(status, 200, "closing an unknown shell is quiet");
    assert!(alive(pid_a));
    // Nor can anyone without a session, or without the CSRF token.
    let (status, body, _) = post(
        p.addr,
        "/dphone.v1.ShellService/WriteShell",
        &[("Content-Type", "application/json")],
        serde_json::json!({ "sessionId": a.id, "data": "" }).to_string().as_bytes(),
    )
    .await;
    assert_eq!(status, 401, "{}", String::from_utf8_lossy(&body));
    let (status, _, _) = post(
        p.addr,
        "/dphone.v1.ShellService/WriteShell",
        &[("Content-Type", "application/json"), ("Cookie", &l.cookie)],
        serde_json::json!({ "sessionId": a.id, "data": "" }).to_string().as_bytes(),
    )
    .await;
    assert_eq!(status, 403);

    // Dropping the stream (the tab closing) kills the shell and what it
    // started, and frees the slot.
    assert_eq!(write(p.addr, &l, &a.id, "sleep 300 &\n").await.0, 200);
    assert_eq!(write(p.addr, &l, &a.id, "echo job=$!=\n").await.0, 200);
    a.until(|t| printed_number(&t.text(), "job").is_some()).await;
    let job = printed_number(&a.text(), "job").unwrap();
    assert!(alive(job));
    drop(a);
    wait_gone(pid_a).await;
    wait_gone(job).await;
    let mut d = Terminal::open(p.addr, &l, 80, 24).await;
    assert!(!d.id.is_empty(), "the slot was freed: {:?}", d.end);

    // CloseShell ends a shell from the server side.
    let (status, _) = call(p.addr, &l, "ShellService/CloseShell", serde_json::json!({ "sessionId": b.id })).await;
    assert_eq!(status, 200);
    b.until(|t| t.exited.is_some()).await;
    assert_eq!(b.exited.clone().unwrap().reason, "closed");
    wait_gone(pid_b).await;

    // The last shell goes the same way when its stream drops.
    let pid_d = shell_pid(p.addr, &l, &mut d).await;
    let (status, v) = write(p.addr, &other, &d.id, "x").await;
    assert_eq!((status, v["code"].as_str()), (404, Some("not_found")));
    drop(d);
    wait_gone(pid_d).await;
}

#[tokio::test]
async fn ssh_keys() {
    let p = phone(false).await;
    let l = login(p.addr).await;
    let file = p.dir.join("nvdata/root/.ssh/authorized_keys");
    let existing = "# kept as written\n\nfrom=\"10.0.0.0/8\",no-pty   ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQC7 options kept\n";
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, existing).unwrap();

    let (status, v) = call(p.addr, &l, "SshService/GetSsh", serde_json::json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(v["status"]["enabled"], serde_json::json!({ "value": true, "source": "SETTING_SOURCE_DEFAULT" }));
    // The existing line's data is not a real RSA key, so it is not listed
    // (and not touched).
    assert_eq!(v["status"]["keys"], serde_json::Value::Null);

    // Add: listed as ssh-keygen describes it; the file gains exactly one line.
    let (status, v) = call(p.addr, &l, "SshService/AddSshKey", serde_json::json!({ "key": format!("  {ED}\n") })).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(
        v["status"]["keys"],
        serde_json::json!([{ "type": "ssh-ed25519", "fingerprint": ED_FP, "comment": "ed@test", "bits": 256 }])
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("{existing}{ED}\n"));
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }

    // Duplicate, malformed, wrong type, and private keys are refused, and
    // the file is unchanged.
    let refused = [
        (ED.to_owned(), "already_exists", "already"),
        ("ssh-ed25519 AAAAC3Nz".to_owned(), "invalid_argument", "not a valid public key"),
        ("ssh-dss AAAAB3NzaC1kc3M= old".to_owned(), "invalid_argument", "Unsupported key type"),
        ("hello there".to_owned(), "invalid_argument", "must start with the key type"),
        (
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU\n-----END OPENSSH PRIVATE KEY-----\n".to_owned(),
            "invalid_argument",
            "private key",
        ),
    ];
    for (key, code, text) in refused {
        let (status, v) = call(p.addr, &l, "SshService/AddSshKey", serde_json::json!({ "key": key })).await;
        assert_ne!(status, 200);
        assert_eq!(v["code"], code, "{v}");
        let message = v["message"].as_str().unwrap();
        assert!(message.contains(text), "{message}");
        assert!(!message.contains("b3Blbn"), "the input is not repeated");
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("{existing}{ED}\n"));

    // A provisioned key is listed as such and cannot be removed here.
    let provisioned = format!(
        "<config><dphone_ssh_authorized_keys><key id=\"1\" value=\"{EC256}\"/></dphone_ssh_authorized_keys></config>"
    );
    p.config.lock().unwrap().apply_fetched(&provisioned).unwrap();
    let (_, v) = call(p.addr, &l, "SshService/GetSsh", serde_json::json!({})).await;
    assert_eq!(
        v["status"]["keys"],
        serde_json::json!([
            { "type": "ssh-ed25519", "fingerprint": ED_FP, "comment": "ed@test", "bits": 256 },
            { "type": "ecdsa-sha2-nistp256", "fingerprint": EC256_FP, "comment": "ec256", "bits": 256, "provisioned": true },
        ])
    );
    let (status, v) = call(p.addr, &l, "SshService/RemoveSshKey", serde_json::json!({ "fingerprint": EC256_FP })).await;
    assert_eq!((status, v["code"].as_str()), (400, Some("failed_precondition")));
    let (status, v) = call(p.addr, &l, "SshService/RemoveSshKey", serde_json::json!({ "fingerprint": "SHA256:nope" })).await;
    assert_eq!((status, v["code"].as_str()), (404, Some("not_found")));

    // Remove: only that key's line goes.
    let (status, v) = call(p.addr, &l, "SshService/RemoveSshKey", serde_json::json!({ "fingerprint": ED_FP })).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["status"]["keys"].as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), existing);

    // Without a session none of it is reachable.
    let (status, _, _) =
        post(p.addr, "/dphone.v1.SshService/AddSshKey", &[("Content-Type", "application/json")], br#"{"key":"x"}"#).await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn ssh_switch_and_firmware_policy() {
    // Without an SshControl the setting is stored in the settings file.
    let p = phone(false).await;
    let l = login(p.addr).await;
    let settings = p.dir.join("nvdata/dphone_settings.txtpb");
    assert!(!settings.exists());
    let (status, v) = call(p.addr, &l, "SshService/SetSshEnabled", serde_json::json!({ "enabled": false })).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["status"]["enabled"], serde_json::json!({ "source": "SETTING_SOURCE_LOCAL" }));
    assert_eq!(v["status"].get("listening"), None);
    assert!(std::fs::read_to_string(&settings).unwrap().contains("ssh_enabled: false"));
    assert!(!p.config.lock().unwrap().config().ssh_enabled().value);
    let (_, v) = call(p.addr, &l, "SshService/SetSshEnabled", serde_json::json!({ "enabled": true })).await;
    assert_eq!(v["status"]["enabled"], serde_json::json!({ "value": true, "source": "SETTING_SOURCE_LOCAL" }));

    // The provisioned-firmware policy, the other setting of ours.
    let (_, v) = call(p.addr, &l, "FirmwareService/GetProvisionedFirmwarePolicy", serde_json::json!({})).await;
    assert_eq!(v["ignore"], serde_json::json!({ "source": "SETTING_SOURCE_DEFAULT" }));
    let (status, v) =
        call(p.addr, &l, "FirmwareService/SetProvisionedFirmwarePolicy", serde_json::json!({ "ignore": true })).await;
    assert_eq!(status, 200);
    assert_eq!(v["ignore"], serde_json::json!({ "value": true, "source": "SETTING_SOURCE_LOCAL" }));
    assert!(std::fs::read_to_string(&settings).unwrap().contains("ignore_provisioned_firmware: true"));

    // A value the server froze cannot be changed here.
    p.config
        .lock()
        .unwrap()
        .apply_fetched(
            r#"<config><setting id="dphone_ssh_enabled" value="0" conflict="freeze"/><setting id="dphone_ignore_provisioned_firmware" value="0" conflict="freeze"/></config>"#,
        )
        .unwrap();
    let (_, v) = call(p.addr, &l, "SshService/GetSsh", serde_json::json!({})).await;
    assert_eq!(v["status"]["enabled"], serde_json::json!({ "source": "SETTING_SOURCE_PROVISIONED", "locked": true }));
    let (status, v) = call(p.addr, &l, "SshService/SetSshEnabled", serde_json::json!({ "enabled": true })).await;
    assert_eq!((status, v["code"].as_str()), (400, Some("failed_precondition")));
    let (status, v) =
        call(p.addr, &l, "FirmwareService/SetProvisionedFirmwarePolicy", serde_json::json!({ "ignore": true })).await;
    assert_eq!((status, v["code"].as_str()), (400, Some("failed_precondition")));

    // With an SshControl, the integrator stores the setting and switches
    // the listener, and reports whether it is listening.
    let p = phone(true).await;
    let l = login(p.addr).await;
    let (_, v) = call(p.addr, &l, "SshService/GetSsh", serde_json::json!({})).await;
    assert_eq!(v["status"]["listening"], true);
    let (status, v) = call(p.addr, &l, "SshService/SetSshEnabled", serde_json::json!({ "enabled": false })).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["status"]["listening"], false);
    assert_eq!(v["status"]["enabled"], serde_json::json!({ "source": "SETTING_SOURCE_LOCAL" }));
    assert!(!p.config.lock().unwrap().config().ssh_enabled().value);
}
