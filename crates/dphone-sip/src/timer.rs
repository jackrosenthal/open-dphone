//! Session timers (RFC 4028): the header forms and the decisions of each
//! role.  The stack carries the headers but has no session-timer logic,
//! so the negotiation, the refresh, and the expiry are ours.
//!
//! Stock is PJSIP with its session-timer module compiled in (`core` holds
//! its `Session-Expires`, `Min-SE`, `refresher`, and 422 diagnostics).  The
//! values here are PJSIP's defaults: timers supported but not required,
//! `Session-Expires: 1800`, and `Min-SE: 90`.

use std::time::Duration;

/// Who sends the refreshes, in the terms of one request: its sender
/// (`uac`) or its receiver (`uas`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresher {
    Uac,
    Uas,
}

/// A `Session-Expires` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionExpires {
    pub secs: u32,
    pub refresher: Option<Refresher>,
}

impl SessionExpires {
    /// Parses `1800;refresher=uac`.
    pub fn parse(value: &str) -> Option<SessionExpires> {
        let mut parts = value.split(';');
        let secs = parts.next()?.trim().parse().ok()?;
        let refresher = parts.find_map(|p| {
            let (k, v) = p.split_once('=')?;
            if !k.trim().eq_ignore_ascii_case("refresher") {
                return None;
            }
            match v.trim().to_ascii_lowercase().as_str() {
                "uac" => Some(Refresher::Uac),
                "uas" => Some(Refresher::Uas),
                _ => None,
            }
        });
        Some(SessionExpires { secs, refresher })
    }

    pub fn render(&self) -> String {
        match self.refresher {
            Some(Refresher::Uac) => format!("{};refresher=uac", self.secs),
            Some(Refresher::Uas) => format!("{};refresher=uas", self.secs),
            None => self.secs.to_string(),
        }
    }
}

/// Whether a `Supported` or `Require` value lists the `timer` extension.
pub fn lists_timer(value: &str) -> bool {
    value.split(',').any(|t| t.trim().eq_ignore_ascii_case("timer"))
}

/// What the two sides agreed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Agreed {
    pub secs: u32,
    /// We send the refreshes; otherwise we watch for them.
    pub we_refresh: bool,
}

impl Agreed {
    /// When the refresher refreshes: half way through the interval.
    pub fn refresh_after(&self) -> Duration {
        Duration::from_secs((self.secs / 2).max(1) as u64)
    }

    /// When the other side gives up on a session that was not refreshed:
    /// the interval less the smaller of 32 seconds and a third of it.
    pub fn expire_after(&self) -> Duration {
        Duration::from_secs((self.secs - (self.secs / 3).min(32)) as u64)
    }
}

/// The outcome of a request of ours (INVITE, re-INVITE, or UPDATE) from
/// the `Session-Expires` of its 2xx.  Without one there is no session
/// timer.  A missing `refresher` means the sender of the request.
pub fn from_response(session_expires: Option<&str>) -> Option<Agreed> {
    let se = SessionExpires::parse(session_expires?)?;
    Some(Agreed { secs: se.secs, we_refresh: se.refresher != Some(Refresher::Uas) })
}

/// What to do with a request of theirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// No session timer for this session.
    None,
    /// Their interval is below our minimum: 422 with this `Min-SE`.
    TooSmall { min_se: u32 },
    /// Put this `Session-Expires` in the 2xx (with `Require: timer` when
    /// `require`).
    Use { header: SessionExpires, require: bool, agreed: Agreed },
}

/// The receiver's side of RFC 4028 section 9.
///
/// - Their interval below our `Min-SE`: refuse with 422, but only when
///   they support the extension (otherwise they could not act on it, and
///   the interval is raised instead).
/// - They ask for a timer: accept their interval, and their choice of
///   refresher; without a choice, they refresh if they support the
///   extension, else we do.
/// - They support timers but did not ask: we ask for our interval and
///   refresh ourselves.
/// - They neither ask nor support it: no timer.
pub fn answer(
    session_expires: Option<&str>,
    min_se: Option<&str>,
    peer_supports: bool,
    our_secs: u32,
    our_min: u32,
) -> Answer {
    let their_min = min_se.and_then(|m| m.split(';').next()?.trim().parse::<u32>().ok()).unwrap_or(0);
    let floor = our_min.max(their_min);
    match session_expires.and_then(SessionExpires::parse) {
        Some(se) => {
            if se.secs < our_min && peer_supports {
                return Answer::TooSmall { min_se: our_min };
            }
            let secs = se.secs.max(floor);
            let refresher = se.refresher.unwrap_or(if peer_supports { Refresher::Uac } else { Refresher::Uas });
            Answer::Use {
                header: SessionExpires { secs, refresher: Some(refresher) },
                // `Require: timer` only when they are the one refreshing
                // (and so must understand it).
                require: peer_supports && refresher == Refresher::Uac,
                agreed: Agreed { secs, we_refresh: refresher == Refresher::Uas },
            }
        }
        None if peer_supports => {
            let secs = our_secs.max(floor);
            Answer::Use {
                header: SessionExpires { secs, refresher: Some(Refresher::Uas) },
                require: false,
                agreed: Agreed { secs, we_refresh: true },
            }
        }
        None => Answer::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_forms() {
        assert_eq!(SessionExpires::parse("1800"), Some(SessionExpires { secs: 1800, refresher: None }));
        let se = SessionExpires::parse(" 600 ; Refresher=UAS ").unwrap();
        assert_eq!(se, SessionExpires { secs: 600, refresher: Some(Refresher::Uas) });
        assert_eq!(se.render(), "600;refresher=uas");
        assert_eq!(SessionExpires::parse("soon"), None);
        assert!(lists_timer("replaces, Timer"));
        assert!(!lists_timer("replaces, 100rel"));
    }

    #[test]
    fn timing() {
        let a = Agreed { secs: 1800, we_refresh: true };
        assert_eq!(a.refresh_after(), Duration::from_secs(900));
        assert_eq!(a.expire_after(), Duration::from_secs(1768));
        // Short sessions give up a third early, not 32 seconds.
        assert_eq!(Agreed { secs: 90, we_refresh: false }.expire_after(), Duration::from_secs(60));
    }

    #[test]
    fn our_request() {
        assert_eq!(from_response(None), None);
        assert_eq!(from_response(Some("1800;refresher=uac")), Some(Agreed { secs: 1800, we_refresh: true }));
        assert_eq!(from_response(Some("300;refresher=uas")), Some(Agreed { secs: 300, we_refresh: false }));
        // No refresher parameter: the sender of the request refreshes.
        assert_eq!(from_response(Some("300")), Some(Agreed { secs: 300, we_refresh: true }));
    }

    #[test]
    fn their_request() {
        // They ask and choose themselves.
        assert_eq!(
            answer(Some("1800;refresher=uac"), Some("90"), true, 1800, 90),
            Answer::Use {
                header: SessionExpires { secs: 1800, refresher: Some(Refresher::Uac) },
                require: true,
                agreed: Agreed { secs: 1800, we_refresh: false },
            }
        );
        // They ask and choose us.
        assert!(matches!(
            answer(Some("600;refresher=uas"), None, true, 1800, 90),
            Answer::Use { require: false, agreed: Agreed { secs: 600, we_refresh: true }, .. }
        ));
        // They ask without choosing: they refresh, since they can.
        assert!(matches!(
            answer(Some("600"), None, true, 1800, 90),
            Answer::Use { require: true, agreed: Agreed { secs: 600, we_refresh: false }, .. }
        ));
        // Too small: refused if they can act on a 422, raised if not.
        assert_eq!(answer(Some("30"), None, true, 1800, 90), Answer::TooSmall { min_se: 90 });
        assert!(matches!(
            answer(Some("30"), None, false, 1800, 90),
            Answer::Use { require: false, agreed: Agreed { secs: 90, we_refresh: true }, .. }
        ));
        // They support timers but did not ask: we ask and refresh, never
        // below their Min-SE.
        assert!(matches!(
            answer(None, Some("3600"), true, 1800, 90),
            Answer::Use { require: false, agreed: Agreed { secs: 3600, we_refresh: true }, .. }
        ));
        // Neither: nothing.
        assert_eq!(answer(None, None, false, 1800, 90), Answer::None);
    }
}
