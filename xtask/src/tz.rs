//! `cargo xtask tz-table`: regenerate `crates/opendeskphone/src/tzdata.tsv`
//! from the build machine's zone database.  Each TZif file ends with the
//! zone's current rule as a POSIX TZ string; that line is all the phone needs.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{Result, repo_root};

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() {
            // `posix` and `right` repeat the main tree.
            if !["posix", "right"].contains(&name.as_str()) {
                walk(base, &path, out)?;
            }
            continue;
        }
        let Ok(data) = fs::read(&path) else { continue };
        if !data.starts_with(b"TZif") {
            continue;
        }
        // The footer is the last line: "\n<rule>\n".
        let Some(body) = data.strip_suffix(b"\n") else { continue };
        let Some(start) = body.iter().rposition(|b| *b == b'\n') else { continue };
        let Ok(rule) = std::str::from_utf8(&body[start + 1..]) else { continue };
        if rule.is_empty() {
            continue;
        }
        let zone = path.strip_prefix(base)?.to_string_lossy().into_owned();
        out.push((zone, rule.to_string()));
    }
    Ok(())
}

#[derive(Debug, clap::Args)]
pub struct TzTable {
    /// The zone database
    #[arg(default_value = "/usr/share/zoneinfo")]
    zoneinfo_dir: PathBuf,
}

pub fn table(args: TzTable) -> Result<()> {
    let base = args.zoneinfo_dir.as_path();
    let mut zones = Vec::new();
    walk(base, base, &mut zones)?;
    zones.sort();
    zones.dedup();
    let text: String = zones.iter().map(|(z, r)| format!("{z}\t{r}\n")).collect();
    let out = repo_root().join("crates/opendeskphone/src/tzdata.tsv");
    fs::write(&out, text)?;
    println!("{} zones written to {}", zones.len(), out.display());
    Ok(())
}
