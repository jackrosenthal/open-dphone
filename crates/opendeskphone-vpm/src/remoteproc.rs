// SPDX-License-Identifier: GPL-2.0-or-later
//! Starting and stopping the DSP through the remoteproc sysfs files.  The
//! driver never starts the DSP by itself: the firmware has to be in
//! `/lib/firmware` first.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const SYSFS_CLASS: &str = "/sys/class/remoteproc";
/// Keeps the DSP running between jobs (the driver otherwise halts it 3 s
/// after the last one, which is pointless once a device is active).
pub const NO_HALT_PARAM: &str = "/sys/module/bcmring_vpm/parameters/no_halt";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Offline,
    Running,
    Crashed,
    Other,
}

#[derive(Clone, Debug)]
pub struct Remoteproc {
    dir: PathBuf,
}

impl Remoteproc {
    /// The DSP's remoteproc: the instance whose name looks like the DSP
    /// node (`30800000.dsp`), or the only one there is.
    pub fn find() -> io::Result<Self> {
        Self::find_in(SYSFS_CLASS)
    }

    pub fn find_in(class_dir: impl AsRef<Path>) -> io::Result<Self> {
        let mut all = Vec::new();
        for e in fs::read_dir(class_dir)? {
            let dir = e?.path();
            if !dir.join("state").exists() {
                continue;
            }
            let name = fs::read_to_string(dir.join("name")).unwrap_or_default();
            if name.contains("dsp") || name.contains("vpm") {
                return Ok(Self { dir });
            }
            all.push(dir);
        }
        match all.len() {
            1 => Ok(Self { dir: all.remove(0) }),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "no DSP remoteproc instance")),
        }
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn state(&self) -> io::Result<State> {
        Ok(match fs::read_to_string(self.dir.join("state"))?.trim() {
            "offline" => State::Offline,
            "running" | "attached" => State::Running,
            "crashed" => State::Crashed,
            _ => State::Other,
        })
    }

    /// Firmware file name, relative to the firmware search path.
    pub fn set_firmware(&self, name: &str) -> io::Result<()> {
        fs::write(self.dir.join("firmware"), name)
    }

    pub fn firmware(&self) -> io::Result<String> {
        Ok(fs::read_to_string(self.dir.join("firmware"))?.trim().to_string())
    }

    /// Loads and starts the DSP; a no-op when it already runs.
    pub fn start(&self) -> io::Result<()> {
        if self.state()? == State::Running {
            return Ok(());
        }
        fs::write(self.dir.join("state"), "start")?;
        self.wait_for(State::Running, Duration::from_secs(5))
    }

    pub fn stop(&self) -> io::Result<()> {
        if self.state()? == State::Offline {
            return Ok(());
        }
        fs::write(self.dir.join("state"), "stop")?;
        self.wait_for(State::Offline, Duration::from_secs(5))
    }

    /// Stop and start: the only way out of a DSP assertion.
    pub fn restart(&self) -> io::Result<()> {
        self.stop()?;
        self.start()
    }

    fn wait_for(&self, want: State, timeout: Duration) -> io::Result<()> {
        let end = Instant::now() + timeout;
        loop {
            if self.state()? == want {
                return Ok(());
            }
            if Instant::now() >= end {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "remoteproc state did not change"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Sets the driver's `no_halt` parameter.
pub fn set_no_halt(on: bool) -> io::Result<()> {
    fs::write(NO_HALT_PARAM, if on { "1" } else { "0" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_reads() {
        let root = std::env::temp_dir().join(format!("opendeskphone-vpm-rproc-{}", std::process::id()));
        let rp = root.join("remoteproc0");
        fs::create_dir_all(&rp).unwrap();
        fs::write(rp.join("name"), "30800000.dsp\n").unwrap();
        fs::write(rp.join("state"), "offline\n").unwrap();
        fs::write(rp.join("firmware"), "dspapp1107_lcc.bin\n").unwrap();
        let r = Remoteproc::find_in(&root).unwrap();
        assert_eq!(r.state().unwrap(), State::Offline);
        assert_eq!(r.firmware().unwrap(), "dspapp1107_lcc.bin");
        // A stopped DSP is not stopped again, nor a running one started:
        // nothing here answers a write to `state`, so a write would end in
        // an error when the wait for the new state times out.
        r.stop().unwrap();
        fs::write(rp.join("state"), "running\n").unwrap();
        r.start().unwrap();
        fs::remove_dir_all(&root).unwrap();
    }
}
