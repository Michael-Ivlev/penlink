//! Pairing code + token store (`%APPDATA%\Penlink\host.json`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const CODE_VALIDITY: Duration = Duration::from_secs(5 * 60);
pub const MAX_CODE_FAILURES: u32 = 5;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS random source unavailable");
    b
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

pub fn new_token() -> String {
    hex(&random_bytes::<32>())
}

/// Uniform 6-digit code, "000000".."999999".
pub fn new_code() -> String {
    loop {
        let b = random_bytes::<4>();
        let n = u32::from_le_bytes(b);
        // reject the biased tail so every code is equally likely
        if n < u32::MAX - (u32::MAX % 1_000_000) {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

/// Compare without early exit on the first differing byte.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenEntry {
    pub token: String,
    pub device_name: String,
    pub created_unix: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreData {
    #[serde(default)]
    pub tokens: Vec<TokenEntry>,
}

#[derive(Debug)]
pub struct TokenStore {
    path: Option<PathBuf>,
    pub data: StoreData,
}

impl TokenStore {
    /// `%APPDATA%\Penlink\host.json` (falls back to the working dir).
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("Penlink").join("host.json")
    }

    pub fn in_memory() -> Self {
        Self { path: None, data: StoreData::default() }
    }

    pub fn load(path: &Path) -> Self {
        let data = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<StoreData>(&s).ok())
            .unwrap_or_default();
        Self { path: Some(path.to_path_buf()), data }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.data).unwrap())?;
        std::fs::rename(&tmp, path)
    }

    pub fn has_devices(&self) -> bool {
        !self.data.tokens.is_empty()
    }

    pub fn add_device(&mut self, device_name: &str) -> String {
        let token = new_token();
        let created_unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.data.tokens.push(TokenEntry {
            token: token.clone(),
            device_name: device_name.chars().take(64).collect(),
            created_unix,
        });
        if let Err(e) = self.save() {
            tracing::warn!("could not save {:?}: {e}", self.path);
        }
        token
    }

    pub fn is_valid_token(&self, token: &str) -> bool {
        let mut ok = false;
        for t in &self.data.tokens {
            ok |= ct_eq(&t.token, token);
        }
        ok
    }
}

/// A one-time pairing code with expiry and a limit on wrong guesses.
#[derive(Debug, Clone)]
pub struct PairingCode {
    code: String,
    expires: Instant,
    failures: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairCheck {
    Ok,
    Wrong,
    Expired,
}

impl PairingCode {
    pub fn generate(now: Instant) -> Self {
        Self::with_code(new_code(), now)
    }

    pub fn with_code(code: String, now: Instant) -> Self {
        Self { code, expires: now + CODE_VALIDITY, failures: 0 }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    /// Past its validity time (as opposed to burned by wrong guesses).
    pub fn timed_out(&self, now: Instant) -> bool {
        now >= self.expires
    }

    pub fn is_live(&self, now: Instant) -> bool {
        now < self.expires && self.failures < MAX_CODE_FAILURES
    }

    /// Check a submitted code. A correct code is NOT consumed here; the caller decides
    /// (the host keeps one code usable for multiple devices until it expires).
    pub fn check(&mut self, submitted: &str, now: Instant) -> PairCheck {
        if now >= self.expires || self.failures >= MAX_CODE_FAILURES {
            return PairCheck::Expired;
        }
        if ct_eq(&self.code, submitted.trim()) {
            PairCheck::Ok
        } else {
            self.failures += 1;
            PairCheck::Wrong
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex() {
        let t = new_token();
        assert_eq!(t.len(), 64);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(t, new_token());
    }

    #[test]
    fn code_is_six_digits() {
        for _ in 0..200 {
            let c = new_code();
            assert_eq!(c.len(), 6);
            assert!(c.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn ct_eq_works() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn pairing_code_lifecycle() {
        let t0 = Instant::now();
        let mut c = PairingCode::with_code("123456".into(), t0);
        assert_eq!(c.check("000000", t0), PairCheck::Wrong);
        assert_eq!(c.check(" 123456 ", t0), PairCheck::Ok);
        assert_eq!(c.check("123456", t0 + CODE_VALIDITY), PairCheck::Expired);
        assert!(!c.is_live(t0 + CODE_VALIDITY));
    }

    #[test]
    fn pairing_code_burns_after_too_many_wrong_guesses() {
        let t0 = Instant::now();
        let mut c = PairingCode::with_code("123456".into(), t0);
        for _ in 0..MAX_CODE_FAILURES {
            assert_eq!(c.check("999999", t0), PairCheck::Wrong);
        }
        assert_eq!(c.check("123456", t0), PairCheck::Expired);
    }

    #[test]
    fn store_roundtrip_and_validation() {
        let dir = std::env::temp_dir().join(format!("penlink-test-{}", new_code()));
        let path = dir.join("host.json");
        let mut s = TokenStore::load(&path);
        assert!(!s.has_devices());
        let tok = s.add_device("S26 Ultra");
        assert!(s.is_valid_token(&tok));
        assert!(!s.is_valid_token("nope"));
        assert!(!s.is_valid_token(""));

        let s2 = TokenStore::load(&path);
        assert!(s2.has_devices());
        assert!(s2.is_valid_token(&tok));
        assert_eq!(s2.data.tokens[0].device_name, "S26 Ultra");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_is_treated_as_empty() {
        let dir = std::env::temp_dir().join(format!("penlink-test-{}", new_code()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("host.json");
        std::fs::write(&path, b"{not json").unwrap();
        let s = TokenStore::load(&path);
        assert!(!s.has_devices());
        let _ = std::fs::remove_dir_all(dir);
    }
}
