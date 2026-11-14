//! The `nvdata` partition: stock files at their stock paths, plus the
//! `open/` directory for state stock does not know about.
//!
//! Rules that keep flashing back to stock safe:
//!
//! - Every write is temp file, `fsync`, rename, `fsync` of the directory.
//! - Files ending in `.tmp` are never read.
//! - Files stock also reads are written only in a form stock parses.
//! - Everything new lives under `open/`, which stock ignores and which
//!   stock's factory reset removes.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

use crate::xml::Element;

/// Realm of the web login digest file.
pub const WEB_REALM: &str = "Digium Phone Admin Login";
/// The only web user stock creates.
pub const WEB_USER: &str = "admin";

/// Files stock's factory reset keeps (relative to the nvdata root).
pub const FACTORY_RESET_KEEP: &[&str] = &[
    "etc/enet.cfg",
    "reboot_reason",
    "phoneservice_lockdown",
    "g729-legacy-support.xml",
    "g729-config.xml",
    "if-setup-debug",
];

/// Handle on an nvdata tree (`/nvdata` on the phone, any directory in tests
/// and in the simulator).
#[derive(Clone, Debug)]
pub struct Nvdata {
    root: PathBuf,
}

/// `etc/enet.cfg`: `KEY=value` lines.  Unknown keys and line order are kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnetCfg {
    entries: Vec<(String, String)>,
}

impl EnetCfg {
    pub fn parse(text: &str) -> Self {
        let entries = text
            .lines()
            .filter_map(|l| l.trim().split_once('='))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        EnetCfg { entries }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: &str) {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value.to_owned(),
            None => self.entries.push((key.to_owned(), value.to_owned())),
        }
    }

    /// `ENETCFG_MAC`, the only place the phone's MAC address is stored.
    pub fn mac(&self) -> Option<[u8; 6]> {
        parse_mac(self.get("ENETCFG_MAC")?)
    }

    pub fn dhcp(&self) -> bool {
        self.get("ENETCFG_DHCP") != Some("0")
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.entries {
            s.push_str(k);
            s.push('=');
            s.push_str(v);
            s.push('\n');
        }
        s
    }
}

/// Parse `aa:bb:cc:dd:ee:ff`.
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = s.trim().split(':');
    for b in &mut mac {
        let p = parts.next()?;
        if p.len() != 2 {
            return None;
        }
        *b = u8::from_str_radix(p, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}

/// Wall-clock time as stock saves it in `systime`: `Nov 12 16:14:43 2026`.
///
/// Stock writes the fields in the phone's local time and restores them as
/// `America/Los_Angeles` time.  This type carries the fields only; the clock
/// restored from them is approximate until SNTP corrects it, exactly as on
/// stock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedTime {
    pub year: u32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

impl SavedTime {
    pub fn parse(text: &str) -> Option<Self> {
        let mut f = text.split_whitespace();
        let month = MONTHS.iter().position(|m| Some(*m) == f.clone().next())? as u32 + 1;
        f.next();
        let day = f.next()?.parse().ok()?;
        let mut hms = f.next()?.split(':');
        let hour = hms.next()?.parse().ok()?;
        let minute = hms.next()?.parse().ok()?;
        let second = hms.next()?.parse().ok()?;
        let year = f.next()?.parse().ok()?;
        let t = SavedTime { year, month, day, hour, minute, second };
        (year >= 1970 && (1..=31).contains(&day) && hour < 24 && minute < 60 && second < 61)
            .then_some(t)
    }

    pub fn to_text(&self) -> String {
        format!(
            "{} {} {:02}:{:02}:{:02} {}\n",
            MONTHS[(self.month as usize - 1) % 12],
            self.day,
            self.hour,
            self.minute,
            self.second,
            self.year
        )
    }

    /// Seconds since the epoch, reading the fields as UTC.
    pub fn to_unix_utc(&self) -> u64 {
        let (y, m) = (self.year as i64, self.month as i64);
        // Days from civil (Howard Hinnant's algorithm).
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + self.day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        (days * 86_400 + (self.hour * 3600 + self.minute * 60 + self.second) as i64).max(0) as u64
    }

    /// The UTC fields for a time in seconds since the epoch.
    pub fn from_unix_utc(t: u64) -> Self {
        let days = (t / 86_400) as i64;
        let rem = (t % 86_400) as u32;
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as u32;
        SavedTime { year, month, day, hour: rem / 3600, minute: rem % 3600 / 60, second: rem % 60 }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The htdigest hash: `MD5(user:realm:password)` in lowercase hex.
pub fn htdigest_ha1(user: &str, realm: &str, password: &str) -> String {
    hex(&Md5::digest(format!("{user}:{realm}:{password}").as_bytes()))
}

/// MD5 of arbitrary data in lowercase hex, the form of the `md5s/` cache.
pub fn md5_hex(data: &[u8]) -> String {
    hex(&Md5::digest(data))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(".tmp")
        && !name.contains(['/', '\\', '\0'])
}

fn bad_name(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("invalid name {name:?}"))
}

impl Nvdata {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Nvdata { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute path of a file given relative to the nvdata root.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    /// Create the directories stock's boot creates, plus ours.
    pub fn ensure_layout(&self) -> io::Result<()> {
        for d in [
            "etc",
            "contact_lists",
            "ringtones",
            "md5s",
            "appdata",
            "certs",
            "clientcerts",
            "root/.ssh",
            "open",
            "open/kv",
        ] {
            fs::create_dir_all(self.path(d))?;
        }
        Ok(())
    }

    // ---- raw file access -------------------------------------------------

    /// Read a file; `Ok(None)` if it does not exist.  `.tmp` files are
    /// refused: stock leaves truncated ones behind.
    pub fn read(&self, rel: &str) -> io::Result<Option<Vec<u8>>> {
        if rel.ends_with(".tmp") {
            return Err(bad_name(rel));
        }
        match File::open(self.path(rel)) {
            Ok(mut f) => {
                let mut v = Vec::new();
                f.read_to_end(&mut v)?;
                Ok(Some(v))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn read_string(&self, rel: &str) -> io::Result<Option<String>> {
        Ok(self.read(rel)?.map(|v| String::from_utf8_lossy(&v).into_owned()))
    }

    /// Atomic write: temp file in the same directory, `fsync`, rename,
    /// `fsync` of the directory.
    pub fn write(&self, rel: &str, data: &[u8]) -> io::Result<()> {
        write_atomic(&self.path(rel), data)
    }

    /// Remove a file; a missing file is not an error.
    pub fn remove(&self, rel: &str) -> io::Result<()> {
        let p = self.path(rel);
        match fs::remove_file(&p) {
            Ok(()) => sync_dir(p.parent().unwrap_or(&self.root)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// Names of the regular files in a directory (no `.tmp` files), sorted.
    pub fn list(&self, rel_dir: &str) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        let rd = match fs::read_dir(self.path(rel_dir)) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(e),
        };
        for ent in rd {
            let ent = ent?;
            let name = ent.file_name().to_string_lossy().into_owned();
            if ent.file_type()?.is_file() && !name.ends_with(".tmp") {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    // ---- XML layers ------------------------------------------------------

    /// Read and parse an XML file.  A missing file is `Ok(None)`; a file that
    /// does not parse is an error the caller can report and skip, as the
    /// stock merge skips it.
    pub fn read_xml(&self, rel: &str) -> io::Result<Option<Element>> {
        match self.read_string(rel)? {
            None => Ok(None),
            Some(t) => Element::parse(&t)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{rel}: {e}"))),
        }
    }

    /// Write an XML file in the stock layout.
    pub fn write_xml(&self, rel: &str, root: &Element) -> io::Result<()> {
        self.write(rel, root.to_document().as_bytes())
    }

    // ---- etc/enet.cfg ----------------------------------------------------

    pub fn enet_cfg(&self) -> io::Result<Option<EnetCfg>> {
        Ok(self.read_string("etc/enet.cfg")?.map(|t| EnetCfg::parse(&t)))
    }

    pub fn write_enet_cfg(&self, cfg: &EnetCfg) -> io::Result<()> {
        self.write("etc/enet.cfg", cfg.to_text().as_bytes())
    }

    /// The phone's MAC address from `ENETCFG_MAC`.
    pub fn mac(&self) -> io::Result<Option<[u8; 6]>> {
        Ok(self.enet_cfg()?.and_then(|c| c.mac()))
    }

    // ---- web login (etc/.passwd, htdigest) -------------------------------

    /// Stored HA1 for a web user, if the digest file has one.
    pub fn web_ha1(&self, user: &str) -> io::Result<Option<String>> {
        let Some(text) = self.read_string("etc/.passwd")? else { return Ok(None) };
        for line in text.lines() {
            let mut f = line.trim().splitn(3, ':');
            if f.next() == Some(user) && f.next() == Some(WEB_REALM) {
                return Ok(f.next().map(|h| h.trim().to_ascii_lowercase()));
            }
        }
        Ok(None)
    }

    /// Check a web login against the digest file.  `Ok(None)` means the file
    /// has no entry for the user (the caller falls back to `login_password`).
    pub fn verify_web_password(&self, user: &str, password: &str) -> io::Result<Option<bool>> {
        Ok(self.web_ha1(user)?.map(|ha1| {
            let ours = htdigest_ha1(user, WEB_REALM, password);
            // Constant-time comparison.
            ours.len() == ha1.len()
                && ours.bytes().zip(ha1.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
        }))
    }

    /// Rewrite the digest file with a new password for `user`, in the exact
    /// form stock writes: one `user:realm:hash` line, no trailing newline,
    /// mode 0755.
    pub fn set_web_password(&self, user: &str, password: &str) -> io::Result<()> {
        let line = format!("{user}:{WEB_REALM}:{}", htdigest_ha1(user, WEB_REALM, password));
        self.write("etc/.passwd", line.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.path("etc/.passwd"), fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    // ---- small state files -----------------------------------------------

    pub fn systime(&self) -> io::Result<Option<SavedTime>> {
        Ok(self.read_string("systime")?.and_then(|t| SavedTime::parse(&t)))
    }

    pub fn write_systime(&self, t: &SavedTime) -> io::Result<()> {
        self.write("systime", t.to_text().as_bytes())
    }

    pub fn reboot_reason(&self) -> io::Result<Option<String>> {
        Ok(self.read_string("reboot_reason")?.map(|s| s.trim().to_owned()))
    }

    /// Stock's form is `<who>: <reason>\n`.
    pub fn write_reboot_reason(&self, who: &str, reason: &str) -> io::Result<()> {
        self.write("reboot_reason", format!("{who}: {reason}\n").as_bytes())
    }

    /// True when a factory reset has been requested for the next boot.
    pub fn factory_reset_requested(&self) -> bool {
        self.exists("factory-reset")
    }

    /// Request a factory reset at the next boot (stock's marker file).
    pub fn request_factory_reset(&self) -> io::Result<()> {
        self.write("factory-reset", b"")
    }

    /// Lines of `root/.ssh/authorized_keys`, without blanks and comments.
    pub fn authorized_keys(&self) -> io::Result<Vec<String>> {
        Ok(self
            .read_string(AUTHORIZED_KEYS)?
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
            .collect())
    }

    pub fn write_authorized_keys(&self, keys: &[String]) -> io::Result<()> {
        let mut text = keys.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        self.write_authorized_keys_text(&text)
    }

    /// Writes the whole of `root/.ssh/authorized_keys` (atomically), with
    /// the modes OpenSSH and Dropbear insist on: 0700 for the directory and
    /// 0600 for the file.
    fn write_authorized_keys_text(&self, text: &str) -> io::Result<()> {
        self.write(AUTHORIZED_KEYS, text.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.path("root/.ssh"), fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(self.path(AUTHORIZED_KEYS), fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Appends one line to `root/.ssh/authorized_keys`, leaving every
    /// existing line (comments, options, blank lines) exactly as it is.
    /// Returns `false`, writing nothing, when a line with the same key is
    /// already there.  `line` must be a key line
    /// ([`parse_authorized_key`] accepts it).
    pub fn add_authorized_key(&self, line: &str) -> io::Result<bool> {
        let line = line.trim();
        let key = parse_authorized_key(line)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not an authorized_keys line"))?;
        let mut text = self.read_string(AUTHORIZED_KEYS)?.unwrap_or_default();
        if text.lines().filter_map(parse_authorized_key).any(|k| k.blob == key.blob) {
            return Ok(false);
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(line);
        text.push('\n');
        self.write_authorized_keys_text(&text)?;
        Ok(true)
    }

    /// Removes the line (or lines) of `root/.ssh/authorized_keys` holding
    /// the key with this base64 blob; every other line is kept exactly as
    /// it is.  Returns whether a line was removed.
    pub fn remove_authorized_key(&self, blob: &str) -> io::Result<bool> {
        let Some(text) = self.read_string(AUTHORIZED_KEYS)? else { return Ok(false) };
        let mut kept = String::with_capacity(text.len());
        let mut removed = false;
        for line in text.split_inclusive('\n') {
            if parse_authorized_key(line).is_some_and(|k| k.blob == blob) {
                removed = true;
            } else {
                kept.push_str(line);
            }
        }
        if removed {
            self.write_authorized_keys_text(&kept)?;
        }
        Ok(removed)
    }

    // ---- md5 cache, contacts, BLF, ringtones -----------------------------

    /// Cached digest for a resource (`md5s/<name>`, 32 hex digits).
    pub fn cached_md5(&self, name: &str) -> io::Result<Option<String>> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        Ok(self.read_string(&format!("md5s/{name}"))?.map(|s| s.trim().to_owned()))
    }

    pub fn write_cached_md5(&self, name: &str, md5: &str) -> io::Result<()> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        self.write(&format!("md5s/{name}"), md5.trim().as_bytes())
    }

    /// A downloaded contact list (`contact_lists/<name>`), raw.
    pub fn contact_list(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        self.read(&format!("contact_lists/{name}"))
    }

    pub fn write_contact_list(&self, name: &str, data: &[u8]) -> io::Result<()> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        self.write(&format!("contact_lists/{name}"), data)
    }

    pub fn contact_list_names(&self) -> io::Result<Vec<String>> {
        self.list("contact_lists")
    }

    /// `blf_items.xml`, the local BLF selection (the merged configuration's
    /// `smart_blf` is the fallback).
    pub fn blf_items(&self) -> io::Result<Option<Element>> {
        self.read_xml("blf_items.xml")
    }

    pub fn write_blf_items(&self, root: &Element) -> io::Result<()> {
        self.write_xml("blf_items.xml", root)
    }

    pub fn ringtone_names(&self) -> io::Result<Vec<String>> {
        self.list("ringtones")
    }

    // ---- open/: state stock does not know about --------------------------

    /// Read a value from the `open/kv/` area (SSH host key, DPMA identity,
    /// trial-boot record, and so on).  Names have no path separators.
    pub fn open_get(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        self.read(&format!("open/kv/{name}"))
    }

    /// Store a value in the `open/kv/` area, mode 0600.
    pub fn open_put(&self, name: &str, data: &[u8]) -> io::Result<()> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        fs::create_dir_all(self.path("open/kv"))?;
        let rel = format!("open/kv/{name}");
        self.write(&rel, data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.path(&rel), fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn open_remove(&self, name: &str) -> io::Result<()> {
        if !valid_name(name) {
            return Err(bad_name(name));
        }
        self.remove(&format!("open/kv/{name}"))
    }

    pub fn open_names(&self) -> io::Result<Vec<String>> {
        self.list("open/kv")
    }

    // ---- factory reset ---------------------------------------------------

    /// Perform the factory reset the way stock's boot does: delete every
    /// file except [`FACTORY_RESET_KEEP`] (and `extra_keep`, paths relative
    /// to the root), remove every directory except the root and `etc`
    /// (directories holding a kept file survive), then remove the marker.
    ///
    /// `open/` is removed like everything else unless the caller lists files
    /// under it in `extra_keep`.
    pub fn factory_reset(&self, extra_keep: &[&str]) -> io::Result<()> {
        let keep: Vec<PathBuf> = FACTORY_RESET_KEEP
            .iter()
            .chain(extra_keep)
            .map(|k| self.path(k))
            .collect();
        reset_dir(&self.root, &keep, true)?;
        fs::create_dir_all(self.path("etc"))?;
        match fs::remove_file(self.path("factory-reset")) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        sync_dir(&self.root)
    }
}

/// Empties `dir` except for kept paths; returns whether `dir` is now empty.
fn reset_dir(dir: &Path, keep: &[PathBuf], is_root: bool) -> io::Result<bool> {
    let mut empty = true;
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        let ft = ent.file_type()?;
        if ft.is_dir() {
            if keep.iter().any(|k| k.starts_with(&p)) || (is_root && p.file_name() == Some("etc".as_ref())) {
                reset_dir(&p, keep, false)?;
                empty = false;
            } else {
                fs::remove_dir_all(&p)?;
            }
        } else if keep.contains(&p) || (is_root && p.file_name() == Some("factory-reset".as_ref())) {
            empty = false;
        } else {
            fs::remove_file(&p)?;
        }
    }
    sync_dir(dir)?;
    Ok(empty)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Write `data` to `path` atomically.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("path has no parent"))?;
    fs::create_dir_all(dir)?;
    let mut name = path.file_name().ok_or_else(|| io::Error::other("path has no file name"))?.to_owned();
    name.push(format!(".{}.tmp", std::process::id()));
    let tmp = dir.join(name);
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        sync_dir(dir)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

const AUTHORIZED_KEYS: &str = "root/.ssh/authorized_keys";

/// The parts of one `authorized_keys` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorizedKey<'a> {
    /// Login options before the key type (`from="..."`, `no-pty`), or empty.
    pub options: &'a str,
    /// `ssh-ed25519`, `ssh-rsa`, `ecdsa-sha2-nistp256`, and so on.
    pub key_type: &'a str,
    /// The key itself, base64, as written.
    pub blob: &'a str,
    pub comment: &'a str,
}

fn looks_like_key_type(token: &str) -> bool {
    (token.starts_with("ssh-") || token.starts_with("ecdsa-sha2-") || token.starts_with("sk-"))
        && token.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'@' | b'.'))
}

/// Splits an `authorized_keys` line into its parts.  `None` for blank
/// lines, comments, and anything that is not a key line.  The key itself is
/// not decoded here.
pub fn parse_authorized_key(line: &str) -> Option<AuthorizedKey<'_>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let first = line.split_ascii_whitespace().next()?;
    let (options, rest) = if looks_like_key_type(first) {
        ("", line)
    } else {
        // Options run to the first whitespace outside double quotes.
        let mut quoted = false;
        let mut escaped = false;
        let mut end = None;
        for (i, c) in line.char_indices() {
            match c {
                _ if escaped => escaped = false,
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                c if c.is_ascii_whitespace() && !quoted => {
                    end = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let end = end?;
        (&line[..end], line[end..].trim_start())
    };
    let (key_type, rest) = rest.split_once(|c: char| c.is_ascii_whitespace())?;
    let rest = rest.trim_start();
    if !looks_like_key_type(key_type) {
        return None;
    }
    let (blob, comment) = match rest.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((b, c)) => (b, c.trim()),
        None => (rest, ""),
    };
    if blob.is_empty() || !blob.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')) {
        return None;
    }
    Some(AuthorizedKey { options, key_type, blob, comment })
}

#[cfg(test)]
mod authorized_keys_tests {
    use super::*;

    const ED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGtq1fDCKW2XS1T4BqRrJ2tR5Yx0b6Hq0c9cJm0C1b2D a@b";
    const RSA: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQC7 old key";

    #[test]
    fn line_parts() {
        let k = parse_authorized_key(ED).unwrap();
        assert_eq!((k.options, k.key_type, k.comment), ("", "ssh-ed25519", "a@b"));
        let with_options = format!("from=\"10.0.0.1, 10.0.0.2\",no-pty {RSA}");
        let k = parse_authorized_key(&with_options).unwrap();
        assert_eq!(k.options, "from=\"10.0.0.1, 10.0.0.2\",no-pty");
        assert_eq!((k.key_type, k.blob, k.comment), ("ssh-rsa", "AAAAB3NzaC1yc2EAAAADAQABAAAAgQC7", "old key"));
        assert!(parse_authorized_key("# comment").is_none());
        assert!(parse_authorized_key("").is_none());
        assert!(parse_authorized_key("ssh-ed25519").is_none());
        assert!(parse_authorized_key("hello world").is_none());
    }

    #[test]
    fn add_and_remove_keep_other_lines_verbatim() {
        let dir = std::env::temp_dir().join(format!("dphone-ak-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let nv = Nvdata::new(&dir);
        let original = format!("# my keys\n\nfrom=\"10.0.0.1\",no-pty   {RSA}\n");
        fs::create_dir_all(dir.join("root/.ssh")).unwrap();
        fs::write(dir.join("root/.ssh/authorized_keys"), &original).unwrap();

        assert!(nv.add_authorized_key(ED).unwrap());
        assert!(!nv.add_authorized_key(ED).unwrap(), "a duplicate is not added");
        let text = fs::read_to_string(dir.join("root/.ssh/authorized_keys")).unwrap();
        assert_eq!(text, format!("{original}{ED}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &str| fs::metadata(dir.join(p)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode("root/.ssh"), 0o700);
            assert_eq!(mode("root/.ssh/authorized_keys"), 0o600);
        }

        let blob = parse_authorized_key(ED).unwrap().blob;
        assert!(nv.remove_authorized_key(blob).unwrap());
        assert!(!nv.remove_authorized_key(blob).unwrap());
        assert_eq!(fs::read_to_string(dir.join("root/.ssh/authorized_keys")).unwrap(), original);
        let _ = fs::remove_dir_all(&dir);
    }
}
