//! The main role against a real server: Asterisk with `res_dpma_rs` (the
//! open DPMA module, <https://github.com/jackrosenthal/dpma-rs>), both on
//! loopback.  The real binary in simulator mode finds its saved server,
//! provisions over DPMA (handshake with a freshly minted identity, token,
//! configuration), writes the stock-format files, and registers the line
//! the configuration gave it; the server's own view is read back through
//! its CLI.  Then the server's configuration changes and it tells the phone,
//! which fetches the new one.
//!
//! Skipped unless `DPHONE_TEST_ASTERISK` names an Asterisk installation
//! (the prefix that holds `bin/asterisk`, the sample configuration in
//! `etc/asterisk`, and the data in `var/lib/asterisk`) and
//! `DPHONE_TEST_ASTERISK_MODULES` a module directory with `res_dpma_rs.so`
//! in it.  With `DPHONE_TEST_REQUIRE_ASTERISK` set (the Nix check sets it,
//! and supplies both: nix/asterisk.nix) it fails instead of skipping.

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

/// The DPMA configuration: one network, and the phone with the simulator's
/// MAC address, whose line is the PJSIP endpoint 1001.  The network covers
/// every address because the simulator has no network of its own: its
/// configuration merge is never told an address, and selects the network
/// as a phone at 0.0.0.0 would.
fn dpma_conf(timezone: &str) -> String {
    format!(
        "[general]\nuserlist_auth=disabled\nconfig_auth=mac\nservice_discovery_enabled=no\n\n\
         [everywhere]\ntype=network\nalias=Everywhere\ncidr=0.0.0.0/0\nregistration_address=127.0.0.1\n\
         registration_port={SIP_PORT}\ntransport=udp\n\n\
         [simphone]\ntype=phone\nmac={MAC}\nnetwork=everywhere\nline=1001\nfull_name=Sim Phone\n\
         timezone={timezone}\ndphone_ssh_enabled=no\n\n\
         [1001]\ntype=line\n"
    )
}

/// The server's configuration: the installation's samples (several modules
/// that `res_dpma_rs` needs do not load without their files), with ours on
/// top.
fn configure(dir: &Path, server: &Server) -> PathBuf {
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
             [1001]\ntype=aor\nmax_contacts=1\n"
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

#[test]
fn provisions_and_registers_against_asterisk() {
    let Some(server) = server() else { return };
    let dir = temp();
    let conf = configure(&dir.path().join("ast"), &server);
    let server_log = dir.path().join("ast/log/messages.log");
    let phone_log = dir.path().join("phone.log");
    let _show = ShowOnPanic(vec![server_log.clone(), phone_log.clone()]);

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
    let _main = KillOnDrop(
        Command::new(BIN)
            .args(["main", "--sim", "--model", "d60", "--no-ssh", "--nvdata"])
            .arg(&nv)
            .arg("--run-dir")
            .arg(&run)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&phone_log).unwrap())
            .spawn()
            .unwrap(),
    );
    let socket = run.join("diag.sock");
    wait_for("the diag socket", Duration::from_secs(30), || socket.exists().then_some(()));

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
    assert!(fetched.contains("<setting id=\"dphone_ssh_enabled\" value=\"0\""), "{fetched}");
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
    assert!(contacts.contains("Objects found: 1"), "{contacts}");

    // The server's view of the DPMA session.
    let sessions = cli(&server, &conf, "digium_phones show sessions").unwrap();
    assert!(sessions.contains("Total active sessions:1"), "{sessions}");
    assert!(sessions.to_ascii_lowercase().contains(&format!("mac:{MAC}")), "{sessions}");

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
}
