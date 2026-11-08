//! Bookkeeping for the virtual monitors Penlink creates: how their GUIDs look (so a leftover
//! one can be recognised) and the file that remembers them across host crashes.
//!
//! The SudoVDA driver can not list its monitors, so every GUID is written to
//! `%APPDATA%\Penlink\vdisplays.json` before the monitor is created and removed from it after
//! the monitor was removed. At start-up the host removes whatever is still listed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Last 8 bytes of every Penlink monitor GUID (`data4`). The first 8 are random.
pub const GUID_MARKER: [u8; 8] = *b"PENLINK!";
/// Serial numbers of Penlink monitors start with this (EDID serial, at most 13 characters).
pub const SERIAL_PREFIX: &str = "PENLINK";
/// Monitor name the driver reports for Penlink monitors (at most 13 characters).
pub const DEVICE_NAME: &str = "Penlink";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

impl Guid {
    /// A new GUID in the Penlink namespace from 8 random bytes.
    pub fn penlink(random: [u8; 8]) -> Guid {
        Guid {
            d1: u32::from_le_bytes([random[0], random[1], random[2], random[3]]),
            d2: u16::from_le_bytes([random[4], random[5]]),
            d3: u16::from_le_bytes([random[6], random[7]]),
            d4: GUID_MARKER,
        }
    }

    pub fn is_penlink(&self) -> bool {
        self.d4 == GUID_MARKER
    }

    /// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
    pub fn format(&self) -> String {
        format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.d1, self.d2, self.d3, self.d4[0], self.d4[1], self.d4[2], self.d4[3], self.d4[4], self.d4[5],
            self.d4[6], self.d4[7]
        )
    }

    pub fn parse(s: &str) -> Option<Guid> {
        let s = s.trim().trim_start_matches('{').trim_end_matches('}');
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 5 || [8, 4, 4, 4, 12].iter().zip(&parts).any(|(n, p)| p.len() != *n) {
            return None;
        }
        let d1 = u32::from_str_radix(parts[0], 16).ok()?;
        let d2 = u16::from_str_radix(parts[1], 16).ok()?;
        let d3 = u16::from_str_radix(parts[2], 16).ok()?;
        let tail = format!("{}{}", parts[3], parts[4]);
        let mut d4 = [0u8; 8];
        for (i, b) in d4.iter_mut().enumerate() {
            *b = u8::from_str_radix(&tail[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Guid { d1, d2, d3, d4 })
    }

    /// Serial number string for the driver: `PENLINK` + 5 hex digits (12 characters).
    pub fn serial(&self) -> String {
        format!("{SERIAL_PREFIX}{:05X}", (self.d1 ^ ((self.d2 as u32) << 8) ^ self.d3 as u32) & 0xF_FFFF)
    }
}

/// The remembered GUIDs, backed by a JSON file `{"version":1,"guids":["..."]}`.
#[derive(Debug)]
pub struct Ledger {
    path: PathBuf,
    guids: BTreeSet<Guid>,
}

pub fn default_path() -> PathBuf {
    // A second host (tests next to a running one) must not clean up or list the other's monitors.
    if let Some(p) = std::env::var_os("PENLINK_VD_LEDGER").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Penlink").join("vdisplays.json")
}

impl Ledger {
    /// Load the file; a missing or damaged file is an empty ledger.
    pub fn load(path: &Path) -> Ledger {
        let mut guids = BTreeSet::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                for g in v.get("guids").and_then(|g| g.as_array()).into_iter().flatten() {
                    // Only our own GUIDs are ever removed.
                    if let Some(g) = g.as_str().and_then(Guid::parse).filter(Guid::is_penlink) {
                        guids.insert(g);
                    }
                }
            }
        }
        Ledger { path: path.to_path_buf(), guids }
    }

    pub fn all(&self) -> Vec<Guid> {
        self.guids.iter().copied().collect()
    }

    pub fn contains(&self, g: &Guid) -> bool {
        self.guids.contains(g)
    }

    pub fn add(&mut self, g: Guid) {
        if self.guids.insert(g) {
            self.save();
        }
    }

    pub fn remove(&mut self, g: &Guid) {
        if self.guids.remove(g) {
            self.save();
        }
    }

    fn save(&self) {
        let list: Vec<String> = self.guids.iter().map(Guid::format).collect();
        let body = serde_json::json!({ "version": 1, "guids": list }).to_string();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &self.path)).is_err() {
            tracing::warn!("could not write {}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("penlink-ledger-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("vdisplays.json")
    }

    #[test]
    fn guid_format_parse_roundtrip() {
        let g = Guid::penlink([1, 2, 3, 4, 5, 6, 7, 8]);
        let s = g.format();
        assert_eq!(s.len(), 36);
        assert_eq!(Guid::parse(&s), Some(g));
        assert_eq!(Guid::parse(&format!("{{{s}}}")), Some(g));
        assert!(g.is_penlink());
        // marker survives the text form: "PENLINK!" = 50 45 4e 4c 49 4e 4b 21
        assert!(s.ends_with("504e-454e4c494e4b21") || s.contains("5045-4e4c494e4b21"));
    }

    #[test]
    fn foreign_and_bad_guids() {
        let g = Guid::parse("dff7fd29-5b75-41d1-9731-b32a17a17104").unwrap();
        assert!(!g.is_penlink());
        assert_eq!(Guid::parse("nope"), None);
        assert_eq!(Guid::parse("dff7fd29-5b75-41d1-9731-b32a17a1710"), None);
        assert_eq!(Guid::parse("zzf7fd29-5b75-41d1-9731-b32a17a17104"), None);
    }

    #[test]
    fn serial_fits_and_has_prefix() {
        let s = Guid::penlink([0xff; 8]).serial();
        assert!(s.starts_with(SERIAL_PREFIX));
        assert!(s.len() <= 13);
        assert!(DEVICE_NAME.len() <= 13);
    }

    #[test]
    fn ledger_persists_and_reloads() {
        let p = tmp("persist");
        let a = Guid::penlink([1; 8]);
        let b = Guid::penlink([2; 8]);
        let mut l = Ledger::load(&p);
        assert!(l.all().is_empty());
        l.add(a);
        l.add(b);
        l.add(a); // duplicate
        assert_eq!(Ledger::load(&p).all().len(), 2);
        l.remove(&a);
        let again = Ledger::load(&p);
        assert_eq!(again.all(), vec![b]);
        assert!(again.contains(&b) && !again.contains(&a));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn ledger_ignores_damage_and_foreign_guids() {
        let p = tmp("damage");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{ not json").unwrap();
        assert!(Ledger::load(&p).all().is_empty());
        // a GUID that is not ours is never touched, even if somebody wrote it into the file
        let mine = Guid::penlink([9; 8]);
        std::fs::write(
            &p,
            format!(r#"{{"version":1,"guids":["{}","dff7fd29-5b75-41d1-9731-b32a17a17104","junk"]}}"#, mine.format()),
        )
        .unwrap();
        assert_eq!(Ledger::load(&p).all(), vec![mine]);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
