//! Web sessions and login throttling.
//!
//! A session is a random token in an `HttpOnly`, `SameSite=Strict` cookie,
//! plus a second random token the page sends back in `X-Csrf-Token` on
//! every authenticated request.  Sessions live in memory only: a restart
//! of the main role logs everyone out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const COOKIE: &str = "dphone_session";
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Most sessions kept; the least recently used is dropped beyond this.
const MAX_SESSIONS: usize = 16;
/// Client addresses tracked for throttling.
const MAX_CLIENTS: usize = 64;
/// Failures allowed before delays start.
const FREE_FAILURES: u32 = 3;
/// Longest lockout.
const MAX_LOCKOUT: Duration = Duration::from_secs(300);
/// A client's failure count is forgotten after this long without one.
const FORGET_AFTER: Duration = Duration::from_secs(900);

#[derive(Clone, Debug)]
pub struct Session {
    pub token: String,
    pub csrf: String,
    pub user: String,
    last_seen: Instant,
}

struct Client {
    failures: u32,
    locked_until: Option<Instant>,
    last_failure: Instant,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Session>,
    clients: HashMap<IpAddr, Client>,
}

pub struct Sessions {
    inner: Mutex<Inner>,
    idle: Duration,
}

/// 128 random bits as hex.
pub fn random_token() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the kernel random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Compares without an early exit on the first differing byte.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Sessions {
    pub fn new(idle: Duration) -> Self {
        Sessions { inner: Mutex::new(Inner::default()), idle }
    }

    /// Opens a session for `user`.
    pub fn create(&self, user: &str) -> Session {
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        let idle = self.idle;
        g.sessions.retain(|_, s| now.duration_since(s.last_seen) < idle);
        while g.sessions.len() >= MAX_SESSIONS {
            let Some(oldest) = g.sessions.values().min_by_key(|s| s.last_seen).map(|s| s.token.clone()) else {
                break;
            };
            g.sessions.remove(&oldest);
        }
        let s = Session { token: random_token(), csrf: random_token(), user: user.to_owned(), last_seen: now };
        g.sessions.insert(s.token.clone(), s.clone());
        s
    }

    /// Looks a session up by cookie token and marks it used.
    pub fn touch(&self, token: &str) -> Option<Session> {
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        let idle = self.idle;
        match g.sessions.get_mut(token) {
            Some(s) if now.duration_since(s.last_seen) < idle => {
                s.last_seen = now;
                Some(s.clone())
            }
            Some(_) => {
                g.sessions.remove(token);
                None
            }
            None => None,
        }
    }

    /// Sessions that have not idled out.
    pub fn count(&self) -> usize {
        let g = self.inner.lock().unwrap();
        let now = Instant::now();
        g.sessions.values().filter(|s| now.duration_since(s.last_seen) < self.idle).count()
    }

    pub fn remove(&self, token: &str) {
        self.inner.lock().unwrap().sessions.remove(token);
    }

    /// Ends every session.
    pub fn clear(&self) {
        self.inner.lock().unwrap().sessions.clear();
    }

    /// Ends every session except `keep` (after a password change).
    pub fn remove_all_except(&self, keep: &str) {
        self.inner.lock().unwrap().sessions.retain(|t, _| t == keep);
    }

    /// Seconds a client must still wait before it may try to log in.
    pub fn locked_for(&self, client: IpAddr) -> Option<Duration> {
        let g = self.inner.lock().unwrap();
        let until = g.clients.get(&client)?.locked_until?;
        until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
    }

    /// Records a failed login; returns the lockout that now applies.
    pub fn record_failure(&self, client: IpAddr) -> Option<Duration> {
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        g.clients.retain(|_, c| now.duration_since(c.last_failure) < FORGET_AFTER);
        if g.clients.len() >= MAX_CLIENTS && !g.clients.contains_key(&client) {
            // Full table (many addresses guessing): drop the stalest entry.
            if let Some(old) = g.clients.iter().min_by_key(|(_, c)| c.last_failure).map(|(k, _)| *k) {
                g.clients.remove(&old);
            }
        }
        let c = g.clients.entry(client).or_insert(Client { failures: 0, locked_until: None, last_failure: now });
        c.failures += 1;
        c.last_failure = now;
        if c.failures >= FREE_FAILURES {
            let exp = (c.failures - FREE_FAILURES).min(16);
            let lock = Duration::from_secs(2u64.saturating_pow(exp)).min(MAX_LOCKOUT);
            c.locked_until = Some(now + lock);
            Some(lock)
        } else {
            None
        }
    }

    pub fn record_success(&self, client: IpAddr) {
        self.inner.lock().unwrap().clients.remove(&client);
    }
}

/// Finds the session cookie in a `Cookie` header value.
pub fn cookie_token(header: &str) -> Option<&str> {
    header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == COOKIE).then_some(value)
    })
}

/// The `Set-Cookie` value for a new session.
pub fn set_cookie(token: &str, secure: bool) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict{}", if secure { "; Secure" } else { "" })
}

/// The `Set-Cookie` value that removes the cookie.
pub fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_lifecycle() {
        let s = Sessions::new(Duration::from_secs(60));
        let a = s.create("admin");
        assert_eq!(a.token.len(), 32);
        assert_ne!(a.token, a.csrf);
        assert!(s.touch(&a.token).is_some());
        assert!(s.touch("nope").is_none());
        let b = s.create("admin");
        s.remove_all_except(&b.token);
        assert!(s.touch(&a.token).is_none());
        assert!(s.touch(&b.token).is_some());
        s.remove(&b.token);
        assert!(s.touch(&b.token).is_none());
    }

    #[test]
    fn idle_expiry() {
        let s = Sessions::new(Duration::from_millis(1));
        let a = s.create("admin");
        std::thread::sleep(Duration::from_millis(5));
        assert!(s.touch(&a.token).is_none());
    }

    #[test]
    fn session_cap() {
        let s = Sessions::new(Duration::from_secs(60));
        let first = s.create("admin");
        for _ in 0..MAX_SESSIONS {
            s.create("admin");
        }
        assert!(s.touch(&first.token).is_none());
    }

    #[test]
    fn throttle() {
        let s = Sessions::new(Duration::from_secs(60));
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(s.record_failure(ip).is_none());
        assert!(s.record_failure(ip).is_none());
        assert_eq!(s.record_failure(ip), Some(Duration::from_secs(1)));
        assert_eq!(s.record_failure(ip), Some(Duration::from_secs(2)));
        assert_eq!(s.record_failure(ip), Some(Duration::from_secs(4)));
        assert!(s.locked_for(ip).is_some());
        assert!(s.locked_for("10.0.0.2".parse().unwrap()).is_none());
        s.record_success(ip);
        assert!(s.locked_for(ip).is_none());
    }

    #[test]
    fn cookies() {
        assert_eq!(cookie_token("a=b; dphone_session=abc; c=d"), Some("abc"));
        assert_eq!(cookie_token("a=b"), None);
        assert!(set_cookie("t", true).ends_with("; Secure"));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
