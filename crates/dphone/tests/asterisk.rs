//! The main role against a real server: Asterisk with `res_dpma_rs` (the
//! open DPMA module, `dpma/` in this repository), both on
//! loopback.  The real binary in simulator mode finds its saved server,
//! provisions over DPMA (handshake with a freshly minted identity, token,
//! configuration), writes the stock-format files, and registers the line
//! the configuration gave it; the server's own view is read back through
//! its CLI.  The phone watches the server's other line, 1002, the way the
//! configuration tells it to (the server's contact list on a free key, one
//! `presence` subscription), and the key's state follows that line as it
//! rings, is answered, and hangs up; 1002 itself is a scripted endpoint
//! here.  Then the server's configuration changes and it tells the phone,
//! which fetches the new one.  A phone that starts again with that
//! configuration verifies its token, gets the configuration's digest in
//! the answer, and does not fetch what it already has.
//!
//! The phone's section also sets the Open DPhone options the server has
//! for the phone's own behavior (`dphone_*`): each arrives as a setting of
//! the fetched configuration and reads back from the merged one.
//!
//! Skipped unless `DPHONE_TEST_ASTERISK` names an Asterisk installation
//! (the prefix that holds `bin/asterisk`, the sample configuration in
//! `etc/asterisk`, and the data in `var/lib/asterisk`) and
//! `DPHONE_TEST_ASTERISK_MODULES` a module directory with `res_dpma_rs.so`
//! in it.  With `DPHONE_TEST_REQUIRE_ASTERISK` set (the Nix check sets it,
//! and supplies both: nix/asterisk.nix) it fails instead of skipping.

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_dphone");

/// The simulator's MAC address (the main role's net subsystem).
const MAC: &str = "000fd3000001";
/// Asterisk's SIP port; the phone itself takes the stock 5060.
const SIP_PORT: u16 = 15060;

struct Server {
    prefix: PathBuf,
    modules: PathBuf,
}

fn server() -> Option<Server> {
    let found = std::env::var_os("DPHONE_TEST_ASTERISK")
        .zip(std::env::var_os("DPHONE_TEST_ASTERISK_MODULES"))
        .map(|(p, m)| Server { prefix: p.into(), modules: m.into() });
    if found.is_none() {
        assert!(
            std::env::var_os("DPHONE_TEST_REQUIRE_ASTERISK").is_none(),
            "DPHONE_TEST_REQUIRE_ASTERISK is set, but DPHONE_TEST_ASTERISK and DPHONE_TEST_ASTERISK_MODULES are not"
        );
        eprintln!("SKIPPED: set DPHONE_TEST_ASTERISK and DPHONE_TEST_ASTERISK_MODULES to run this test");
    }
    found
}

/// A temp directory whose paths are short enough for a Unix socket.
fn temp() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("da").tempdir().unwrap();
    if dir.path().as_os_str().len() < 60 {
        return dir;
    }
    tempfile::Builder::new().prefix("da").tempdir_in("/tmp").unwrap()
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Prints files when the test that holds it fails.
struct ShowOnPanic(Vec<PathBuf>);

impl Drop for ShowOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            for p in &self.0 {
                eprintln!("--- {} ---\n{}", p.display(), std::fs::read_to_string(p).unwrap_or_default());
            }
        }
    }
}

fn wait_for<T>(what: &str, timeout: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = check() {
            return v;
        }
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // A file copied from a read-only installation is read-only itself.
    let _ = std::fs::remove_file(path);
    std::fs::write(path, text).unwrap();
}

/// The Open DPhone options of the phone's section: the option and its value
/// in `res_digium_phone.conf`, and the value of the setting of the same
/// name in the configuration the server builds.  None of them acts on the
/// machine the simulator runs on: the simulator leaves the host's network
/// alone, and has no UART to give to a console.
const DPHONE_OPTIONS: [(&str, &str, &str); 8] = [
    ("dphone_ssh_enabled", "no", "0"),
    ("dphone_metrics_enabled", "no", "0"),
    ("dphone_serial_console", "yes", "1"),
    ("dphone_static_address", "192.0.2.20/24", "192.0.2.20/24"),
    ("dphone_static_gateway", "192.0.2.1", "192.0.2.1"),
    ("dphone_static_dns", "192.0.2.53, 192.0.2.54", "192.0.2.53,192.0.2.54"),
    ("dphone_disabled_apps", "calendar, parking", "calendar,parking"),
    ("dphone_idle_softkeys", "call_log,contacts,redial,menu", "call_log,contacts,redial,menu"),
];

/// The DPMA configuration: one network, and the phone with the simulator's
/// MAC address, whose line is the PJSIP endpoint 1001.  The network covers
/// every address because the simulator has no network of its own: its
/// configuration merge is never told an address, and selects the network
/// as a phone at 0.0.0.0 would.
///
/// A second phone has line 1002.  The server makes a contact of every
/// phone's line (the document it serves as `file://default`: "Internal
/// Extensions", each contact with `subscribe_to` naming the hint the server
/// creates for the line), and `blf_contact_group` with
/// `blf_unused_linekeys` puts those contacts on the simulator's free keys.
///
/// The `dphone_*` options are [`DPHONE_OPTIONS`].
fn dpma_conf(timezone: &str) -> String {
    let options: String = DPHONE_OPTIONS.iter().map(|(option, value, _)| format!("{option}={value}\n")).collect();
    format!(
        "[general]\nuserlist_auth=disabled\nconfig_auth=mac\nservice_discovery_enabled=no\n\n\
         [everywhere]\ntype=network\nalias=Everywhere\ncidr=0.0.0.0/0\nregistration_address=127.0.0.1\n\
         registration_port={SIP_PORT}\ntransport=udp\n\n\
         [simphone]\ntype=phone\nmac={MAC}\nnetwork=everywhere\nline=1001\nfull_name=Sim Phone\n\
         timezone={timezone}\n{options}contact=file://default\n\
         blf_contact_group=Internal Extensions\nblf_unused_linekeys=yes\n\n\
         [otherphone]\ntype=phone\nmac=000fd3000002\nnetwork=everywhere\nline=1002\nfull_name=Bob Watched\n\n\
         [1001]\ntype=line\n\n[1002]\ntype=line\n"
    )
}

/// The device of line 1002: enough of a SIP endpoint to be called.  It
/// answers an INVITE with 180, and with 200 (and an SDP answer) once
/// `answer` is called; it says 200 to everything else (the BYE that ends
/// the call) and nothing to an ACK.
struct Callee {
    socket: UdpSocket,
    /// The 200 to the INVITE that is ringing.
    pending: std::sync::Arc<std::sync::Mutex<Option<(String, std::net::SocketAddr)>>>,
}

impl Callee {
    fn start() -> Callee {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let pending = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (rx, slot) = (socket.try_clone().unwrap(), pending.clone());
        let port = socket.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok((n, from)) = rx.recv_from(&mut buf) {
                let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                let method = text.split(' ').next().unwrap_or("");
                if method == "ACK" || method.starts_with("SIP/") {
                    continue;
                }
                // The headers a response repeats; the To gets our tag.
                let mut echo = String::new();
                for line in text.lines().skip(1).take_while(|l| !l.is_empty()) {
                    let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
                    match name.as_str() {
                        "via" | "v" | "from" | "f" | "call-id" | "i" | "cseq" => echo.push_str(&format!("{line}\r\n")),
                        "to" | "t" if line.contains("tag=") => echo.push_str(&format!("{line}\r\n")),
                        "to" | "t" => echo.push_str(&format!("{line};tag=callee1002\r\n")),
                        _ => {}
                    }
                }
                let contact = format!("Contact: <sip:1002@127.0.0.1:{port}>\r\n");
                let reply = |status: &str, body: &str| {
                    let kind = if body.is_empty() { "" } else { "Content-Type: application/sdp\r\n" };
                    format!("SIP/2.0 {status}\r\n{echo}{contact}{kind}Content-Length: {}\r\n\r\n{body}", body.len())
                };
                if method == "INVITE" {
                    let sdp = format!(
                        "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
                         m=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n",
                        port + 2 - port % 2
                    );
                    *slot.lock().unwrap() = Some((reply("200 OK", &sdp), from));
                    let _ = rx.send_to(reply("180 Ringing", "").as_bytes(), from);
                } else {
                    let _ = rx.send_to(reply("200 OK", "").as_bytes(), from);
                }
            }
        });
        Callee { socket, pending }
    }

    fn port(&self) -> u16 {
        self.socket.local_addr().unwrap().port()
    }

    /// Answers the ringing call; false when none rings yet.
    fn answer(&self) -> bool {
        let Some((ok, to)) = self.pending.lock().unwrap().take() else { return false };
        self.socket.send_to(ok.as_bytes(), to).unwrap();
        true
    }
}

/// The server's configuration: the installation's samples (several modules
/// that `res_dpma_rs` needs do not load without their files), with ours on
/// top.
fn configure(dir: &Path, server: &Server, callee_port: u16) -> PathBuf {
    let etc = dir.join("etc");
    std::fs::create_dir_all(&etc).unwrap();
    for entry in std::fs::read_dir(server.prefix.join("etc/asterisk")).expect("the sample configuration") {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            std::fs::copy(entry.path(), etc.join(entry.file_name())).unwrap();
        }
    }
    let d = dir.display();
    write(
        &etc.join("asterisk.conf"),
        &format!(
            "[directories]\nastetcdir => {d}/etc\nastmoddir => {}\nastvarlibdir => {d}/lib\nastdbdir => {d}/lib\n\
             astkeydir => {d}/lib\nastdatadir => {}\nastagidir => {d}/lib/agi-bin\nastspooldir => {d}/spool\n\
             astrundir => {d}/run\nastlogdir => {d}/log\nastsbindir => {d}/sbin\n",
            server.modules.display(),
            server.prefix.join("var/lib/asterisk").display()
        ),
    );
    // Everything loads by default, as on an ordinary server; the modules
    // that only complain here (hardware, other servers, and the other
    // provisioning module) are left out, as in dpma-rs's own test instance.
    let noload = [
        "chan_unistim", "chan_mgcp", "chan_skinny", "chan_motif", "res_xmpp", "chan_console", "chan_alsa", "chan_oss",
        "res_hep", "res_hep_pjsip", "res_hep_rtcp", "res_pjsip_phoneprov_provider", "res_phoneprov", "res_smdi",
        "res_config_ldap", "res_snmp", "cdr_radius", "cel_radius", "chan_dahdi", "res_calendar", "pbx_dundi",
    ];
    let mut modules_conf = String::from("[modules]\nautoload=yes\n");
    for m in noload {
        modules_conf.push_str(&format!("noload => {m}.so\n"));
    }
    write(&etc.join("modules.conf"), &modules_conf);
    write(
        &etc.join("logger.conf"),
        "[general]\n[logfiles]\nmessages.log => notice,warning,error,verbose,dpma\n",
    );
    write(
        &etc.join("pjsip.conf"),
        &format!(
            "[global]\ntype=global\ndefault_outbound_endpoint=dpma_endpoint\n\n\
             [transport-udp]\ntype=transport\nprotocol=udp\nbind=127.0.0.1:{SIP_PORT}\n\n\
             [dpma_endpoint]\ntype=endpoint\ncontext=default\ndisallow=all\nallow=ulaw\n\n\
             [1001]\ntype=endpoint\ncontext=default\ndisallow=all\nallow=ulaw\nauth=1001\naors=1001\n\
             callerid=\"Alice Example\" <1001>\n\n\
             [1001]\ntype=auth\nauth_type=userpass\nusername=1001\npassword=secret1001\n\n\
             [1001]\ntype=aor\nmax_contacts=1\n\n\
             [1002]\ntype=endpoint\ncontext=default\ndisallow=all\nallow=ulaw\naors=1002\n\
             callerid=\"Bob Watched\" <1002>\n\n\
             [1002]\ntype=aor\ncontact=sip:1002@127.0.0.1:{callee_port}\n"
        ),
    );
    write(&etc.join("res_digium_phone.conf"), &dpma_conf("America/Denver"));
    for sub in ["lib/keys", "spool", "run", "log", "sbin"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    etc.join("asterisk.conf")
}

/// One command on the running server's console; `None` while the server
/// does not answer.
fn cli(server: &Server, conf: &Path, command: &str) -> Option<String> {
    let asterisk = server.prefix.join("bin/asterisk");
    let out = Command::new(asterisk).arg("-C").arg(conf).arg("-rx").arg(command).output().unwrap();
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn diag(socket: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(BIN).args(args).env("DPHONE_DIAG_SOCKET", socket).output().unwrap();
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The `id` of a `BlfEntry` in the store's debug output.
fn id_of(entry: &str) -> String {
    let rest = entry.split("id: \"").nth(1).expect("an id");
    rest.split('"').next().unwrap().to_string()
}

#[test]
fn provisions_and_registers_against_asterisk() {
    let Some(server) = server() else { return };
    let dir = temp();
    let callee = Callee::start();
    let conf = configure(&dir.path().join("ast"), &server, callee.port());
    let server_log = dir.path().join("ast/log/messages.log");
    let phone_log = dir.path().join("phone.log");
    // The phone's second run, at the end.
    let phone_log_2 = dir.path().join("phone-2.log");
    // What the phone last said of its watched contacts and keys.
    let blf_dump = dir.path().join("blf.txt");
    let _show = ShowOnPanic(vec![server_log.clone(), phone_log.clone(), phone_log_2.clone(), blf_dump.clone()]);

    // The server, in the foreground without a console.
    let _asterisk = KillOnDrop(
        Command::new(server.prefix.join("bin/asterisk"))
            .arg("-f")
            .arg("-C")
            .arg(&conf)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("asterisk starts"),
    );
    wait_for("Asterisk to boot", Duration::from_secs(120), || cli(&server, &conf, "core waitfullybooted"));
    let module = cli(&server, &conf, "module show like res_dpma_rs").unwrap();
    assert!(module.contains("res_dpma_rs.so") && module.contains("Running"), "{module}");
    let phone = cli(&server, &conf, "digium_phones show phone simphone").unwrap();
    assert!(phone.contains(MAC), "{phone}");

    // The phone: the simulator, with the server saved as stock saves it.
    let nv = dir.path().join("nv");
    let run = dir.path().join("run");
    write(
        &nv.join("configurator.xml"),
        &format!("<config>\n    <setting id=\"config_server_url\" value=\"sip:proxy@127.0.0.1:{SIP_PORT}\" />\n</config>\n"),
    );
    let socket = run.join("diag.sock");
    let start_phone = |log: &Path| {
        let phone = KillOnDrop(
            Command::new(BIN)
                // A D65: enough line keys for both of the server's contacts.
                .args(["main", "--sim", "--model", "d65", "--no-ssh", "--nvdata"])
                .arg(&nv)
                .arg("--run-dir")
                .arg(&run)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(log).unwrap())
                .spawn()
                .unwrap(),
        );
        wait_for("the diag socket", Duration::from_secs(30), || diag(&socket, &["ping"]).map(|_| ()));
        phone
    };
    let main = start_phone(&phone_log);

    // Provisioned: by MAC, with no user to choose.
    let provisioning = wait_for("provisioning", Duration::from_secs(60), || {
        diag(&socket, &["state", "provisioning"]).filter(|s| s.contains("phase: Provisioned"))
    });
    assert!(provisioning.contains("attached: true"), "{provisioning}");
    assert!(provisioning.contains(&format!("127.0.0.1:{SIP_PORT}")), "{provisioning}");

    // What the server sent is on disk in stock's files, with the Open
    // DPhone setting the phone section carries.
    let fetched = std::fs::read_to_string(nv.join("fetched-config.xml")).expect("fetched-config.xml");
    assert!(fetched.contains("username=\"1001\""), "{fetched}");
    for (option, _, value) in DPHONE_OPTIONS {
        assert!(fetched.contains(&format!("<setting id=\"{option}\" value=\"{value}\"")), "{option}: {fetched}");
        let got = diag(&socket, &["config", "get", option]).expect("config get");
        assert!(got.contains(value), "{option}: {got}");
    }
    // The two that the simulator acts on: `/metrics` is off, and the
    // serial console has the UART instead of a headset adapter.
    for said in ["metrics disabled (metrics_enabled)", "the serial console has the UART (serial_console is on)"] {
        wait_for(said, Duration::from_secs(30), || diag(&socket, &["log", "-n", "5000"]).filter(|l| l.contains(said)));
    }
    // Without `active_locale` the server sends no locale setting, where it
    // used to send an empty one.
    assert!(!fetched.contains("id=\"locale\""), "{fetched}");
    assert!(nv.join("md5s/config").exists());
    let configurator = std::fs::read_to_string(nv.join("configurator.xml")).unwrap();
    assert!(configurator.contains("config_server_token"), "{configurator}");
    let zone = diag(&socket, &["config", "get", "time_zone"]).expect("config get");
    assert!(zone.contains("America/Denver"), "{zone}");

    // Registered: the account from that configuration, on our side and on
    // the server's.
    let accounts = wait_for("registration", Duration::from_secs(60), || {
        diag(&socket, &["state", "accounts"]).filter(|s| s.contains("Registered {"))
    });
    assert!(accounts.contains("user: \"1001\""), "{accounts}");
    let contacts = wait_for("the contact on the server", Duration::from_secs(30), || {
        cli(&server, &conf, "pjsip show contacts").filter(|s| s.contains("1001/sip:1001@127.0.0.1"))
    });
    // (The other one is the scripted endpoint's, from the configuration.)
    assert!(contacts.contains("Objects found: 2"), "{contacts}");

    // The server's view of the DPMA session.
    let sessions = cli(&server, &conf, "digium_phones show sessions").unwrap();
    assert!(sessions.contains("Total active sessions:1"), "{sessions}");
    assert!(sessions.to_ascii_lowercase().contains(&format!("mac:{MAC}")), "{sessions}");

    // The watched line.  The server's contacts are on the free line keys
    // (the simulator's own line among them), and 1002 has one subscription
    // on the server: `presence`.
    let blf = |want: &str| {
        let what = format!("line 1002 to be {want}");
        wait_for(&what, Duration::from_secs(30), || {
            let presence = diag(&socket, &["state", "presence"])?;
            let keys = diag(&socket, &["state", "keys"]).unwrap_or_default();
            write(&blf_dump, &format!("{presence}\n{keys}"));
            let entry = presence.split("BlfEntry {").find(|e| e.contains("auto_hint_1002@"))?;
            entry.contains(&format!("state: {want},")).then(|| entry.to_string())
        })
    };
    let entry = blf("Idle");
    assert!(entry.contains("number: \"1002\"") && entry.contains("subscribed: true"), "{entry}");
    let subscriptions = cli(&server, &conf, "pjsip show subscriptions inbound").unwrap();
    assert!(subscriptions.contains("auto_hint_1002/presence"), "{subscriptions}");
    assert!(!subscriptions.contains("/dialog"), "{subscriptions}");
    // It is on a key of its own.
    let keys = diag(&socket, &["state", "keys"]).unwrap();
    assert!(keys.split("LineKey {").any(|k| k.contains("Blf(") && k.contains(&id_of(&entry))), "{keys}");
    // The server calls 1002: it rings, answers, and the server hangs up.
    cli(&server, &conf, "channel originate PJSIP/1002 application Wait 60").unwrap();
    blf("Ringing");
    wait_for("the call to ring at 1002", Duration::from_secs(10), || callee.answer().then_some(()));
    blf("InUse");
    cli(&server, &conf, "channel request hangup all").unwrap();
    blf("Idle");

    // A push: the phone's configuration changes on the server, the server
    // tells the phone over the session, and the phone fetches it.
    write(&conf.with_file_name("res_digium_phone.conf"), &dpma_conf("America/Chicago"));
    let reloaded = cli(&server, &conf, "module reload res_dpma_rs").unwrap();
    assert!(reloaded.contains("reloaded successfully"), "{reloaded}");
    wait_for("the server to tell the phone", Duration::from_secs(30), || {
        cli(&server, &conf, "digium_phones reconfigure phone simphone").filter(|s| s.contains("will reconfigure"))
    });
    wait_for("the pushed configuration", Duration::from_secs(60), || {
        diag(&socket, &["config", "get", "time_zone"]).filter(|z| z.contains("America/Chicago"))
    });
    let fetched = std::fs::read_to_string(nv.join("fetched-config.xml")).unwrap();
    assert!(fetched.contains("America/Chicago"), "{fetched}");

    // A phone that starts with the configuration it has: the server
    // answers its VerifyRequest with the digest of the configuration, the
    // digest is the cached one, and nothing is fetched.
    let digest = nv.join("md5s/config");
    let written = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
    let (digest_before, fetched_before) = (written(&digest), written(&nv.join("fetched-config.xml")));
    let cached = std::fs::read_to_string(&digest).unwrap();
    assert_eq!(cached.len(), 32, "{cached}");
    drop(main);
    let _ = std::fs::remove_file(&socket);
    let _main = start_phone(&phone_log_2);
    let provisioning = wait_for("provisioning after a restart", Duration::from_secs(60), || {
        diag(&socket, &["state", "provisioning"]).filter(|s| s.contains("phase: Provisioned"))
    });
    assert!(provisioning.contains(&format!("config_md5: \"{cached}\"")), "{provisioning}");
    assert_eq!(written(&digest), digest_before, "the configuration was fetched again");
    assert_eq!(written(&nv.join("fetched-config.xml")), fetched_before, "the configuration was fetched again");
    let zone = diag(&socket, &["config", "get", "time_zone"]).expect("config get");
    assert!(zone.contains("America/Chicago"), "{zone}");
}
