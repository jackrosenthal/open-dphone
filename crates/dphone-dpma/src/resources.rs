//! The files a provisioned configuration names: contact lists, ringtones,
//! the BLF items file, the datasets file, the wallpaper picture, and the
//! palette file that goes with it.  They are fetched after a
//! configuration is applied and kept where stock keeps them, so stock
//! finds them after a flash back and this firmware finds stock's.
//!
//! What stock does:
//!
//! | Configuration | File under `/nvdata` | Digest name |
//! | --- | --- | --- |
//! | `<contacts id url md5>` | `contact_lists/<id>` | `<id>` |
//! | `<ringtones><tones><tone id url md5>` | `ringtones/<id>.sln16` | `<id>` |
//! | `<smart_blf><blf_items url md5>` | `blf_items.xml` | `blf_items` |
//! | `<smart_blf><datasets url md5>` | `datasets.xml` | `datasets` |
//! | `<setting id="wallpaper_file" url md5>` | `user_wallpaper.png` | `wallpaper_file` |
//!
//! And one that only this firmware has, beside the wallpaper picture (the
//! colors of every screen while that picture is the wallpaper; see
//! `proto/dphone/palette/v1/palette.proto`):
//!
//! | Configuration | File under `/nvdata` | Digest name |
//! | --- | --- | --- |
//! | `<setting id="dphone_palette_file" url md5>` | `open/wallpaper_palette.txtpb` | `dphone_palette_file` |
//!
//! In a file name, `%`, `.`, and `/` of an `id` are written `%25`, `%2E`,
//! and `%2F`.  `md5s/<digest name>` holds the MD5 of the file as it was
//! downloaded, 32 hex digits.  A resource is downloaded again unless the
//! configuration gives an `md5` equal to that digest (letter case aside)
//! and the file exists; an entry without `md5` is downloaded on every
//! pass.  An `http://` or `https://` URL is fetched directly, and a
//! `file://NAME` URL with a FileRequest on the DPMA session.  (Stock hands
//! every URL to curl first and sends the FileRequest when curl fails; for
//! a URL that curl can fetch, a server finds no `file://` name in that
//! request and refuses it, so that second step is left out here.)
//! Ringtones share a budget of 1 MiB.  Contact lists, ringtones, the
//! wallpaper picture, and the palette file are removed with their digests
//! when the configuration no longer names them;
//! `local_contacts.xml`, `serialized_contacts.xml`, and ringtones with
//! `user_upload` in the name are never touched.
//!
//! Two differences from stock, both on purpose.  A file is removed only
//! when this mechanism downloaded it (it has a digest): this firmware lets
//! the user keep contact lists of their own in the same directory.  And a
//! download that fails leaves the previous file in place, where stock
//! deletes a contact list before asking for the new one.
//!
//! A server can also name app packages (`<appconfig id><url url md5/>`).
//! Those are stock's JavaScript apps, which cannot run on this firmware:
//! each is skipped with a log entry.

use crate::link::DpmaError;
use md5::{Digest, Md5};
use std::sync::Arc;

/// The ringtone budget (stock: 0x100000 bytes for the whole directory).
pub const RINGTONE_BUDGET: usize = 0x10_0000;
/// The most one resource may be.  Stock has no limit for contact lists;
/// this one is ours, for a phone with little memory and a small flash.
pub const MAX_RESOURCE: usize = 0x10_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ContactList,
    Ringtone,
    BlfItems,
    Datasets,
    /// The wallpaper picture of a color model (a PNG file).
    Wallpaper,
    /// The palette file for the wallpaper picture.
    Palette,
}

/// The single-file kinds: (kind, file, digest name).
const SINGLE: [(Kind, &str, &str); 4] = [
    (Kind::BlfItems, "blf_items.xml", "blf_items"),
    (Kind::Datasets, "datasets.xml", "datasets"),
    (Kind::Wallpaper, "user_wallpaper.png", "wallpaper_file"),
    (Kind::Palette, "open/wallpaper_palette.txtpb", "dphone_palette_file"),
];

/// The most a palette file may be (a complete one is under 1 KiB).
pub const MAX_PALETTE: usize = 16 * 1024;

/// One file the configuration names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub kind: Kind,
    /// The element's `id` (unused for the kinds that are one file).
    pub id: String,
    pub url: String,
    pub md5: Option<String>,
}

/// An app package the server pushes (a stock JavaScript app).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedApp {
    pub id: String,
    pub url: String,
}

/// Everything the merged configuration names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub resources: Vec<Resource>,
    pub apps: Vec<PushedApp>,
}

/// The nvdata tree, by path relative to its root.  Writes are atomic.
/// Writing and removing are slow on flash: [`sync`] calls them on a
/// blocking thread.
pub trait Files: Send + Sync + 'static {
    fn read(&self, rel: &str) -> Option<Vec<u8>>;
    fn write(&self, rel: &str, data: &[u8]) -> Result<(), String>;
    fn remove(&self, rel: &str);
    /// The plain files in a directory, by name.
    fn list(&self, dir: &str) -> Vec<String>;
}

/// Where resources come from.
#[async_trait::async_trait]
pub trait Fetch: Send + Sync {
    /// `file://NAME`: a FileRequest on the session.
    async fn file(&self, name: &str) -> Result<Vec<u8>, DpmaError>;
    /// An `http://` or `https://` URL; fails when the body is longer than
    /// `max` bytes.
    async fn http(&self, url: &str, max: usize) -> Result<Vec<u8>, String>;
}

/// What one pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Downloaded (whether or not the content was new).
    pub fetched: usize,
    /// Skipped because the digest matched.
    pub matched: usize,
    pub failed: usize,
    /// Files removed because the configuration no longer names them.
    pub removed: usize,
    /// Some file on disk is different now.
    pub changed: bool,
}

/// An `id` as stock spells it in a file name.
pub fn file_name(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        match c {
            '%' => out.push_str("%25"),
            '.' => out.push_str("%2E"),
            '/' => out.push_str("%2F"),
            c => out.push(c),
        }
    }
    out
}

fn md5_hex(data: &[u8]) -> String {
    Md5::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

impl Resource {
    /// Whether stock would act on the entry: contact lists and ringtones
    /// need an `id`, and everything needs a URL.
    fn usable(&self) -> bool {
        !self.url.is_empty() && (!self.id.is_empty() || !matches!(self.kind, Kind::ContactList | Kind::Ringtone))
    }

    /// The name of the digest in `md5s/`.
    pub fn digest_name(&self) -> String {
        match self.kind {
            Kind::ContactList | Kind::Ringtone => file_name(&self.id),
            kind => SINGLE.iter().find(|s| s.0 == kind).map_or_else(String::new, |s| s.2.into()),
        }
    }

    /// The file, relative to the nvdata root.
    pub fn path(&self) -> String {
        match self.kind {
            Kind::ContactList => format!("contact_lists/{}", file_name(&self.id)),
            Kind::Ringtone => format!("ringtones/{}.sln16", file_name(&self.id)),
            kind => SINGLE.iter().find(|s| s.0 == kind).map_or_else(String::new, |s| s.1.into()),
        }
    }
}

const KEPT_CONTACT_LISTS: [&str; 2] = ["local_contacts.xml", "serialized_contacts.xml"];

fn digest(files: &dyn Files, name: &str) -> Option<String> {
    files.read(&format!("md5s/{name}")).map(|d| String::from_utf8_lossy(&d).trim().to_string()).filter(|d| !d.is_empty())
}

/// What earlier passes downloaded and the configuration no longer names:
/// each file with the name of its digest.
fn stale(wanted: &[&Resource], files: &dyn Files) -> Vec<(String, String)> {
    let named = |kind: Kind, path: &str| wanted.iter().any(|r| r.kind == kind && r.path() == path);
    let mut out = Vec::new();
    let mut discard = |path: String, digest_name: &str| out.push((path, digest_name.to_string()));
    for name in files.list("contact_lists") {
        let path = format!("contact_lists/{name}");
        if KEPT_CONTACT_LISTS.contains(&name.as_str()) || named(Kind::ContactList, &path) {
            continue;
        }
        if digest(files, &name).is_some() {
            discard(path, &name);
        }
    }
    for name in files.list("ringtones") {
        let path = format!("ringtones/{name}");
        if name.contains("user_upload") || named(Kind::Ringtone, &path) {
            continue;
        }
        let stem = name.strip_suffix(".sln16").unwrap_or(&name);
        if digest(files, stem).is_some() {
            discard(path, stem);
        }
    }
    for (kind, path, digest_name) in SINGLE {
        if !wanted.iter().any(|r| r.kind == kind) && digest(files, digest_name).is_some() {
            discard(path.to_string(), digest_name);
        }
    }
    out
}

/// Runs a flash operation off the async threads.
async fn blocking<T: Send + 'static>(files: &Arc<dyn Files>, f: impl FnOnce(&dyn Files) -> T + Send + 'static) -> Result<T, String> {
    let files = files.clone();
    tokio::task::spawn_blocking(move || f(files.as_ref())).await.map_err(|e| e.to_string())
}

async fn download(r: &Resource, fetch: &dyn Fetch, max: usize) -> Result<Vec<u8>, String> {
    let lower = r.url.to_ascii_lowercase();
    let data = if lower.starts_with("file://") {
        fetch.file(&r.url["file://".len()..]).await.map_err(|e| e.to_string())?
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        fetch.http(&r.url, max).await?
    } else {
        return Err("unsupported URL scheme (file, http, and https are supported)".into());
    };
    if data.len() > max {
        return Err(format!("{} bytes, and {max} are allowed", data.len()));
    }
    Ok(data)
}

/// The log entry for an app package the server pushes.
pub fn skipped_app_notice(app: &PushedApp) -> String {
    format!(
        "dpma: skipping the app {:?} the server pushes ({}): it is a stock JavaScript app, which cannot run on this firmware",
        app.id, app.url
    )
}

/// Says in the log that the pushed app packages are skipped.  Nothing is
/// downloaded for them.
pub fn log_skipped_apps(apps: &[PushedApp]) {
    for app in apps {
        log::info!("{}", skipped_app_notice(app));
    }
}

/// Brings the files on disk in line with what the configuration names
/// (its `resources`; see [`log_skipped_apps`] for its `apps`).
pub async fn sync(wanted: &Wanted, files: &Arc<dyn Files>, fetch: &dyn Fetch) -> Report {
    let mut report = Report::default();
    let usable: Vec<&Resource> = wanted.resources.iter().filter(|r| r.usable()).collect();
    for (path, digest_name) in stale(&usable, files.as_ref()) {
        log::info!("dpma: removing {path}: the configuration no longer names it");
        let removed = blocking(files, move |f| {
            f.remove(&path);
            f.remove(&format!("md5s/{digest_name}"));
        })
        .await;
        if removed.is_ok() {
            report.removed += 1;
            report.changed = true;
        }
    }
    let files_ref: &dyn Files = files.as_ref();

    // What the ringtones on disk leave of their budget; a ringtone that is
    // replaced gives its bytes back first.
    let ringtone_bytes = |files: &dyn Files, except: &str| -> usize {
        files
            .list("ringtones")
            .iter()
            .filter(|n| !n.contains("user_upload"))
            .map(|n| format!("ringtones/{n}"))
            .filter(|p| p != except)
            .map(|p| files.read(&p).map_or(0, |d| d.len()))
            .sum()
    };

    let mut done: Vec<String> = Vec::new();
    for r in usable {
        let (path, name) = (r.path(), r.digest_name());
        // A file named twice is fetched once; the first entry wins.
        if done.contains(&path) {
            continue;
        }
        done.push(path.clone());
        let on_disk = files_ref.read(&path);
        let cached = digest(files_ref, &name);
        if let (Some(want), Some(have), true) = (r.md5.as_deref().filter(|m| !m.is_empty()), cached.as_deref(), on_disk.is_some())
            && want.eq_ignore_ascii_case(have)
        {
            log::debug!("dpma: {path}: the digest matches; not downloaded");
            report.matched += 1;
            continue;
        }
        let max = match r.kind {
            Kind::Ringtone => RINGTONE_BUDGET.saturating_sub(ringtone_bytes(files_ref, &path)).min(MAX_RESOURCE),
            Kind::Palette => MAX_PALETTE,
            _ => MAX_RESOURCE,
        };
        let data = match download(r, fetch, max).await {
            Ok(d) => d,
            Err(e) => {
                log::warn!("dpma: cannot fetch {} for {path}: {e}", r.url);
                report.failed += 1;
                continue;
            }
        };
        report.fetched += 1;
        let (sum, len) = (md5_hex(&data), data.len());
        if on_disk.as_deref() != Some(data.as_slice()) {
            let to = path.clone();
            if let Err(e) = blocking(files, move |f| f.write(&to, &data)).await.and_then(|r| r) {
                log::warn!("dpma: cannot write {path}: {e}");
                report.failed += 1;
                continue;
            }
            log::info!("dpma: {path} updated from {} ({len} bytes)", r.url);
            report.changed = true;
        }
        if cached.as_deref() != Some(sum.as_str()) {
            let to = format!("md5s/{name}");
            if let Err(e) = blocking(files, move |f| f.write(&to, sum.as_bytes())).await.and_then(|r| r) {
                log::warn!("dpma: cannot write md5s/{name}: {e}");
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_as_stock_spells_them() {
        assert_eq!(file_name("0"), "0");
        assert_eq!(file_name("a.b/c%d"), "a%2Eb%2Fc%25d");
        let r = |kind, id: &str| Resource { kind, id: id.into(), url: "file://x".into(), md5: None };
        assert_eq!(r(Kind::ContactList, "0").path(), "contact_lists/0");
        assert_eq!(r(Kind::ContactList, "team.xml").path(), "contact_lists/team%2Exml");
        assert_eq!(r(Kind::ContactList, "team.xml").digest_name(), "team%2Exml");
        assert_eq!(r(Kind::Ringtone, "Chimes").path(), "ringtones/Chimes.sln16");
        assert_eq!(r(Kind::Ringtone, "Chimes").digest_name(), "Chimes");
        assert_eq!((r(Kind::BlfItems, "").path(), r(Kind::BlfItems, "").digest_name()), ("blf_items.xml".into(), "blf_items".into()));
        assert_eq!((r(Kind::Datasets, "").path(), r(Kind::Datasets, "").digest_name()), ("datasets.xml".into(), "datasets".into()));
        assert!(!r(Kind::ContactList, "").usable());
        assert!(r(Kind::BlfItems, "").usable());
        // The wallpaper picture has stock's name.
        assert_eq!((r(Kind::Wallpaper, "").path(), r(Kind::Wallpaper, "").digest_name()), ("user_wallpaper.png".into(), "wallpaper_file".into()));
        assert!(r(Kind::Wallpaper, "").usable() && r(Kind::Palette, "").usable());
    }

    #[derive(Default)]
    struct Mem(std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>);

    impl Files for Mem {
        fn read(&self, rel: &str) -> Option<Vec<u8>> {
            self.0.lock().unwrap().get(rel).cloned()
        }
        fn write(&self, rel: &str, data: &[u8]) -> Result<(), String> {
            self.0.lock().unwrap().insert(rel.into(), data.to_vec());
            Ok(())
        }
        fn remove(&self, rel: &str) {
            self.0.lock().unwrap().remove(rel);
        }
        fn list(&self, dir: &str) -> Vec<String> {
            let prefix = format!("{dir}/");
            self.0.lock().unwrap().keys().filter_map(|k| k.strip_prefix(&prefix)).filter(|n| !n.contains('/')).map(str::to_string).collect()
        }
    }

    /// A server with files by name (`file://`) and by URL, counting what
    /// it is asked for.
    #[derive(Default)]
    struct Server {
        files: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
        asked: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Fetch for Server {
        async fn file(&self, name: &str) -> Result<Vec<u8>, DpmaError> {
            self.asked.lock().unwrap().push(name.into());
            self.files.lock().unwrap().get(name).cloned().ok_or_else(|| DpmaError::Protocol(format!("no file {name}")))
        }
        async fn http(&self, url: &str, max: usize) -> Result<Vec<u8>, String> {
            self.asked.lock().unwrap().push(url.into());
            let data = self.files.lock().unwrap().get(url).cloned().ok_or("404")?;
            if data.len() > max { Err(format!("more than {max} bytes")) } else { Ok(data) }
        }
    }

    #[tokio::test]
    async fn a_wallpaper_picture_and_its_palette_file() {
        let files: Arc<dyn Files> = Arc::new(Mem::default());
        let server = Server::default();
        let serve = |name: &str, data: &[u8]| server.files.lock().unwrap().insert(name.into(), data.to_vec());
        let has = |rel: &str| files.read(rel);
        serve("wall.png", b"\x89PNG picture");
        serve("http://pbx/files/wall.txtpb", b"primary: \"#1F6E4E\"\n");
        let r = |kind, url: &str, md5: Option<&str>| Resource { kind, id: String::new(), url: url.into(), md5: md5.map(str::to_string) };
        let both = Wanted {
            resources: vec![r(Kind::Wallpaper, "file://wall.png", None), r(Kind::Palette, "http://pbx/files/wall.txtpb", None)],
            apps: vec![],
        };
        // Fetched, kept where they belong, with their digests.
        let report = sync(&both, &files, &server).await;
        assert_eq!((report.fetched, report.failed, report.removed, report.changed), (2, 0, 0, true));
        assert_eq!(has("user_wallpaper.png").as_deref(), Some(&b"\x89PNG picture"[..]));
        assert_eq!(has("open/wallpaper_palette.txtpb").as_deref(), Some(&b"primary: \"#1F6E4E\"\n"[..]));
        assert_eq!(has("md5s/wallpaper_file").unwrap(), md5_hex(b"\x89PNG picture").into_bytes());
        assert_eq!(has("md5s/dphone_palette_file").unwrap(), md5_hex(b"primary: \"#1F6E4E\"\n").into_bytes());
        // Without `md5` they are asked for again each pass; the same
        // content changes nothing.
        let report = sync(&both, &files, &server).await;
        assert_eq!((report.fetched, report.changed), (2, false));
        // With a matching `md5` they are not asked for.
        let (png_md5, palette_md5) = (md5_hex(b"\x89PNG picture"), md5_hex(b"primary: \"#1F6E4E\"\n"));
        let pinned = Wanted {
            resources: vec![
                r(Kind::Wallpaper, "file://wall.png", Some(&png_md5.to_uppercase())),
                r(Kind::Palette, "http://pbx/files/wall.txtpb", Some(&palette_md5)),
            ],
            apps: vec![],
        };
        server.asked.lock().unwrap().clear();
        let report = sync(&pinned, &files, &server).await;
        assert_eq!((report.matched, report.fetched), (2, 0));
        assert!(server.asked.lock().unwrap().is_empty());
        // A new palette file takes the old one's place.
        serve("http://pbx/files/wall.txtpb", b"light: true\n");
        let report = sync(&both, &files, &server).await;
        assert!(report.changed);
        assert_eq!(has("open/wallpaper_palette.txtpb").as_deref(), Some(&b"light: true\n"[..]));
        // A palette file over its limit is refused; the one on the phone
        // stays.
        serve("http://pbx/files/wall.txtpb", &vec![b'#'; MAX_PALETTE + 1]);
        let report = sync(&both, &files, &server).await;
        assert_eq!((report.fetched, report.failed), (1, 1));
        assert_eq!(has("open/wallpaper_palette.txtpb").as_deref(), Some(&b"light: true\n"[..]));
        // The palette file is no longer named: it goes, with its digest;
        // the picture stays.
        let picture_only = Wanted { resources: vec![r(Kind::Wallpaper, "file://wall.png", None)], apps: vec![] };
        let report = sync(&picture_only, &files, &server).await;
        assert_eq!((report.removed, report.changed), (1, true));
        assert_eq!((has("open/wallpaper_palette.txtpb"), has("md5s/dphone_palette_file")), (None, None));
        assert!(has("user_wallpaper.png").is_some());
        // Neither is named: the picture goes too.
        let report = sync(&Wanted::default(), &files, &server).await;
        assert_eq!(report.removed, 1);
        assert_eq!((has("user_wallpaper.png"), has("md5s/wallpaper_file")), (None, None));
        // A picture that this mechanism did not download (it has no
        // digest: a stock firmware's upload, say) is left alone.
        files.write("user_wallpaper.png", b"someone's own").unwrap();
        let report = sync(&Wanted::default(), &files, &server).await;
        assert_eq!(report.removed, 0);
        assert!(has("user_wallpaper.png").is_some());
    }

    #[test]
    fn pushed_apps_are_named_in_the_log() {
        let notice = skipped_app_notice(&PushedApp { id: "queues".into(), url: "file://queues.zip".into() });
        assert!(notice.contains("\"queues\"") && notice.contains("file://queues.zip"), "{notice}");
    }
}
