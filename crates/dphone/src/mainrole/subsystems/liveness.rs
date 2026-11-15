//! Per-subsystem liveness (plan section 5) and the last kernel crash log.
//!
//! **Liveness.**  Subsystems with a loop of their own record a heartbeat in
//! the state store.  One that has beaten before and then stops for
//! [`STALE_AFTER`] is marked degraded in the health state (shown by
//! `dphone health`, the web UI, and the log) until it beats again.  Nothing
//! is restarted on that evidence alone: the init role restarts the whole
//! main role when its own heartbeat stops.
//!
//! **Crash log.**  With `ramoops` in the kernel, what the kernel printed
//! before a crash survives the reboot in `/sys/fs/pstore`.  At start those
//! records are moved to `/nvdata/open/crash/` (replacing the previous ones)
//! and reported; `dphone crashlog` prints them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dphone_state::{HealthStatus, Subsystem};
use log::{error, info, warn};

use crate::diag::Output;
use crate::mainrole::Context;

/// Silence after which a subsystem counts as stuck.
pub const STALE_AFTER: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(5);
/// Most that is kept of one crash record.
const RECORD_LIMIT: u64 = 256 * 1024;
const PSTORE: &str = "/sys/fs/pstore";

async fn watch(ctx: Context) {
    // What each stuck subsystem's status was before we marked it.
    let mut marked: HashMap<Subsystem, HealthStatus> = HashMap::new();
    loop {
        tokio::time::sleep(CHECK_EVERY).await;
        // This loop is the core's own sign of life.
        ctx.store.heartbeat(Subsystem::Core);
        let stale = ctx.store.stale_subsystems(STALE_AFTER.as_millis() as u64);
        for s in &stale {
            if !marked.contains_key(s) {
                error!("liveness: {s:?} has not reported for {} s", STALE_AFTER.as_secs());
                let before = ctx
                    .store
                    .read::<dphone_state::Health, _>(|h| h.subsystems.iter().find(|e| e.subsystem == *s).map(|e| e.status.clone()))
                    .unwrap_or(HealthStatus::Ok);
                marked.insert(*s, before);
                ctx.store.set_health(*s, HealthStatus::Degraded("not responding".into()));
            }
        }
        let recovered: Vec<Subsystem> = marked.keys().filter(|s| !stale.contains(s)).copied().collect();
        for s in recovered {
            info!("liveness: {s:?} is responding again");
            if let Some(before) = marked.remove(&s) {
                // Only undo our own mark.
                let ours = ctx.store.read::<dphone_state::Health, _>(|h| {
                    h.subsystems.iter().any(|e| e.subsystem == s && e.status == HealthStatus::Degraded("not responding".into()))
                });
                if ours {
                    ctx.store.set_health(s, before);
                }
            }
        }
    }
}

fn crash_dir(ctx: &Context) -> PathBuf {
    ctx.paths.open_dir().join("crash")
}

/// Move crash records out of pstore.  Returns how many were saved.
pub fn collect(pstore: &Path, into: &Path) -> std::io::Result<usize> {
    let mut records: Vec<PathBuf> = match std::fs::read_dir(pstore) {
        // Only `dmesg-*` records are written at a crash.  The console record
        // (`console-ramoops-0`) is present after every warm restart.
        Ok(d) => d
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("dmesg-")))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    if records.is_empty() {
        return Ok(0);
    }
    records.sort();
    // New records replace the old set.
    let _ = std::fs::remove_dir_all(into);
    std::fs::create_dir_all(into)?;
    let mut saved = 0;
    for record in records {
        let Some(name) = record.file_name() else { continue };
        use std::io::Read;
        let mut data = Vec::new();
        std::fs::File::open(&record)?.take(RECORD_LIMIT).read_to_end(&mut data)?;
        crate::paths::write_atomic(&into.join(name), &data, 0o600)?;
        // Removing the record frees its space in the persistent store.
        let _ = std::fs::remove_file(&record);
        saved += 1;
    }
    Ok(saved)
}

/// Print the crash records the kernel left from before this boot
#[derive(Debug, clap::Parser)]
#[command(name = "crashlog")]
pub struct CrashlogCommand {}

pub fn start(ctx: &Context) {
    tokio::spawn(watch(ctx.clone()));

    if !ctx.sim && ctx.recovery.is_none() {
        match collect(Path::new(PSTORE), &crash_dir(ctx)) {
            Ok(0) => {}
            Ok(n) => warn!("the kernel left {n} crash record(s) from before this boot; see 'dphone crashlog'"),
            Err(e) => warn!("crash records: {e}"),
        }
    }
    let dir = crash_dir(ctx);
    ctx.diag.register(move |_: CrashlogCommand, out: Output| {
        let dir = dir.clone();
        async move {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            if files.is_empty() {
                out.out("no crash records\n").await;
                return 0;
            }
            for f in files {
                out.out(&format!("==> {} <==\n", f.file_name().unwrap_or_default().to_string_lossy())).await;
                match std::fs::read(&f) {
                    Ok(data) => out.out(&String::from_utf8_lossy(&data)).await,
                    Err(e) => out.err(&format!("{e}\n")).await,
                }
                out.out("\n").await;
            }
            0
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_records_move_and_replace() {
        let dir = tempfile::tempdir().unwrap();
        let (pstore, saved) = (dir.path().join("pstore"), dir.path().join("crash"));
        // No pstore at all: nothing to do.
        assert_eq!(collect(&pstore, &saved).unwrap(), 0);
        std::fs::create_dir_all(&pstore).unwrap();
        assert_eq!(collect(&pstore, &saved).unwrap(), 0);
        std::fs::write(pstore.join("dmesg-ramoops-0"), b"Oops: first").unwrap();
        std::fs::write(pstore.join("dmesg-ramoops-1"), b"earlier lines").unwrap();
        assert_eq!(collect(&pstore, &saved).unwrap(), 2);
        assert_eq!(std::fs::read(saved.join("dmesg-ramoops-0")).unwrap(), b"Oops: first");
        assert!(std::fs::read_dir(&pstore).unwrap().next().is_none(), "records are removed from pstore");
        // The console record alone is not a crash.
        std::fs::write(pstore.join("console-ramoops-0"), b"boot messages").unwrap();
        assert_eq!(collect(&pstore, &saved).unwrap(), 0);
        std::fs::remove_file(pstore.join("console-ramoops-0")).unwrap();
        // A later crash replaces the saved set.
        std::fs::write(pstore.join("dmesg-ramoops-0"), b"Oops: second").unwrap();
        assert_eq!(collect(&pstore, &saved).unwrap(), 1);
        assert_eq!(std::fs::read(saved.join("dmesg-ramoops-0")).unwrap(), b"Oops: second");
        assert!(!saved.join("dmesg-ramoops-1").exists());
    }
}
