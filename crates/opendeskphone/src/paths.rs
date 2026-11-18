//! Filesystem locations.  Everything that differs between a phone and the
//! host simulator is derived from here.

use std::path::{Path, PathBuf};

/// Environment variable naming the diagnostics socket for the `dphone`
/// client (`--diag-socket`); set for the shells the main role starts.
pub const DIAG_SOCKET_ENV: &str = "DPHONE_DIAG_SOCKET";

/// Default runtime directory on a phone.
pub const RUN_DIR: &str = "/run/opendeskphone";

/// Default nvdata mount point on a phone.
pub const NVDATA: &str = "/nvdata";

/// Where the image keeps the built-in ringtones.
pub const RINGTONES_DIR: &str = "/usr/share/opendeskphone/ringtones";

/// Where the image keeps the alarm tones.
pub const ALARM_TONES_DIR: &str = "/usr/share/opendeskphone/alarm-tones";

#[derive(Clone, Debug)]
pub struct Paths {
    /// The nvdata volume (stock layout).
    pub nvdata: PathBuf,
    /// Volatile runtime state (`/run/opendeskphone`).
    pub run: PathBuf,
    /// The built-in ringtones ([`RINGTONES_DIR`]).
    pub ringtones: PathBuf,
    /// The alarm tones ([`ALARM_TONES_DIR`]).
    pub alarm_tones: PathBuf,
    /// The staged built-in apps the simulator runs.  A phone's are where
    /// the app manager looks for them (`None`).
    pub apps: Option<PathBuf>,
}

impl Default for Paths {
    fn default() -> Self {
        Paths::new(NVDATA, RUN_DIR)
    }
}

impl Paths {
    /// The paths of a phone with this nvdata and runtime directory.
    pub fn new(nvdata: impl Into<PathBuf>, run: impl Into<PathBuf>) -> Self {
        Paths {
            nvdata: nvdata.into(),
            run: run.into(),
            ringtones: PathBuf::from(RINGTONES_DIR),
            alarm_tones: PathBuf::from(ALARM_TONES_DIR),
            apps: None,
        }
    }

    /// Our own state directory inside nvdata; stock ignores it.
    pub fn own_dir(&self) -> PathBuf {
        self.nvdata.join("opendeskphone")
    }

    pub fn diag_socket(&self) -> PathBuf {
        self.run.join("diag.sock")
    }

    pub fn ssh_host_key(&self) -> PathBuf {
        self.own_dir().join("ssh_host_ed25519_key")
    }

    /// The keys added on the phone, in `opendeskphone/`: the stock firmware
    /// overwrites the stock location with Digium's keys at every boot.
    pub fn authorized_keys(&self) -> PathBuf {
        self.nvdata.join(opendeskphone_config::nvdata::AUTHORIZED_KEYS_FILE)
    }

    /// Home directory for shell sessions.
    pub fn home(&self) -> PathBuf {
        self.nvdata.join("root")
    }

    pub fn systime(&self) -> PathBuf {
        self.nvdata.join("systime")
    }

    pub fn reboot_reason(&self) -> PathBuf {
        self.nvdata.join("reboot_reason")
    }

    /// Present while the serial console has UART A; only then does the
    /// init role run a shell on `/dev/ttyAMA0`.  Absent, the UART is the EHS
    /// headset adapter's (the default), and nothing is written to it.
    pub fn serial_console(&self) -> PathBuf {
        self.run.join("serial-console")
    }

    /// Written by the init role when nvdata could not be mounted.
    pub fn recovery_flag(&self) -> PathBuf {
        self.run.join("recovery")
    }
}

/// Socket path a diagnostic client should use.
/// Flag file carried by images that must leave stock-owned nvdata files
/// exactly as they found them (an image for trying the firmware on a
/// phone that also runs stock).
pub const NO_STOCK_WRITES_FLAG: &str = "/etc/opendeskphone/no-stock-writes";

/// False when this image must not write files that stock also owns
/// (`systime`, `reboot_reason`, `config.xml`, and so on).  Our own directory
/// `/nvdata/opendeskphone/` is always writable.
pub fn stock_writes_allowed() -> bool {
    !Path::new(NO_STOCK_WRITES_FLAG).exists()
}

/// The diagnostics socket of a phone's main role.
pub fn default_diag_socket() -> PathBuf {
    Path::new(RUN_DIR).join("diag.sock")
}

/// Write `data` to `path` by temp file, fsync, rename (section 7 write rule).
pub fn write_atomic(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.new"));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        // The rename itself must reach the flash: on JFFS2 the new
        // directory entry otherwise sits in the write buffer, and a reset
        // (e.g., the watchdog during a trial boot) loses it.
        std::fs::File::open(dir)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
