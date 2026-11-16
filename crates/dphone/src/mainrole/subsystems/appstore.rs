//! The apps' store of keys and values on the PBX (the app interface
//! `pbx-storage`), over DPMA's `AppDataRequest`.  The firmware keeps the
//! user's theme there too (`themesync`).
//!
//! The server knows the phone by its configuration token, so the store is
//! in reach while the DPMA session is attached and the phone provisioned.
//! Once for each session, the phone says hello: that names the apps whose changes it wants to
//! hear of, and the answer says who the phone's user is.  Nothing is kept
//! to send later: a request that cannot reach the server is answered
//! `unreachable`, and the apps are told when the store is in reach again.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dphone_apps::services::{
    Completion, PbxStore, StoreEntry, StoreOp, StoreReply, StoreScope, StoreSignal, StoreUser,
};
use dphone_dpma::appdata::{self, Entry, Op, Outcome, Scope};
use dphone_dpma::{DpmaError, DpmaHandle};
use dphone_state::{Event, ProvisionPhase, Provisioning, Store, Topic};
use log::{debug, info};

use super::telephony::Telephony;

/// After a hello that the server did not answer, requests are answered
/// `unreachable` for this long without asking again: a server without
/// app data (the official DPMA) answers none, and each would otherwise
/// keep its app waiting for the whole timeout.
const HELLO_RETRY: Duration = Duration::from_secs(60);
/// The longest the phone waits before it says hello again to a server
/// that did not answer.
const HELLO_RETRY_MAX: Duration = Duration::from_secs(3600);

type Subscriber = Arc<dyn Fn(StoreSignal) + Send + Sync>;

#[derive(Default)]
struct State {
    /// The server answered the hello of the present session.
    online: bool,
    user: Option<StoreUser>,
    /// The apps whose changes the app host wants.
    watch: Vec<String>,
    subscriber: Option<Subscriber>,
    /// The last hello that failed, and why.
    failed: Option<(Instant, String)>,
}

/// [`PbxStore`] for the app host.
#[derive(Clone)]
pub struct AppStore {
    telephony: Telephony,
    runtime: tokio::runtime::Handle,
    state: Arc<Mutex<State>>,
    /// Notified each time the store comes into reach.
    reached: Arc<tokio::sync::Notify>,
}

fn scope(s: StoreScope) -> Scope {
    match s {
        StoreScope::Shared => Scope::Shared,
        StoreScope::User => Scope::User,
    }
}

fn entry(e: Entry) -> StoreEntry {
    StoreEntry {
        key: e.key,
        value: e.value,
        version: e.version,
        modified: e.modified,
        author_id: e.author_id,
        author_name: e.author_name,
    }
}

fn user(u: appdata::User) -> StoreUser {
    StoreUser { id: u.id, name: u.name, extension: u.extension }
}

/// The request document for an app's request.
fn op(request: StoreOp) -> Op {
    match request {
        StoreOp::Get { app, scope: s, key } => Op::Get { app, scope: scope(s), key },
        StoreOp::List { app, scope: s, prefix, after } => Op::List { app, scope: scope(s), prefix, after },
        StoreOp::Set { app, scope: s, key, value, expect } => Op::Set { app, scope: scope(s), key, value, expect },
        StoreOp::Remove { app, scope: s, key, expect } => Op::Remove { app, scope: scope(s), key, expect },
    }
}

/// What an app gets for the server's answer.
fn reply(outcome: Outcome) -> StoreReply {
    match outcome {
        Outcome::Entry(e) => StoreReply::Entry(e.map(entry)),
        Outcome::Entries { entries, more } => StoreReply::Entries(entries.into_iter().map(entry).collect(), more),
        Outcome::Stored(e) => StoreReply::Stored(entry(e)),
        Outcome::Removed => StoreReply::Removed,
        Outcome::Conflict(e) => StoreReply::Conflict(e.map(entry)),
        Outcome::Error { message, .. } => StoreReply::Refused(message),
        Outcome::Hello => StoreReply::Refused("the server answered another request".into()),
    }
}

/// Why a request did not get an answer, for the app.
fn unreachable(e: &DpmaError) -> String {
    match e {
        DpmaError::Timeout => "the PBX did not answer; it may keep no app data".to_string(),
        other => other.to_string(),
    }
}

impl AppStore {
    pub fn new(telephony: Telephony, runtime: tokio::runtime::Handle) -> AppStore {
        AppStore { telephony, runtime, state: Arc::new(Mutex::new(State::default())), reached: Arc::default() }
    }

    /// Notified each time the store comes into reach (the first answer
    /// of a DPMA session), for the firmware's use of the store; the app
    /// host has [`StoreSignal::Online`].
    pub fn reached(&self) -> Arc<tokio::sync::Notify> {
        self.reached.clone()
    }

    fn dpma(&self) -> Option<DpmaHandle> {
        self.telephony.dpma.lock().unwrap().clone().filter(DpmaHandle::attached)
    }

    fn signal(&self, signal: StoreSignal) {
        let subscriber = self.state.lock().unwrap().subscriber.clone();
        if let Some(f) = subscriber {
            f(signal);
        }
    }

    fn offline(&self) {
        self.state.lock().unwrap().online = false;
    }

    /// Tells the server which apps the phone watches and learns who the
    /// user is.  The first answer of a session puts the store in reach.
    async fn hello(&self, dpma: &DpmaHandle) -> Result<(), String> {
        let watch = self.state.lock().unwrap().watch.clone();
        let result = match dpma.app_data(&Op::Hello { watch }).await {
            Ok(appdata::Response { user: Some(u), outcome: Outcome::Hello }) => Ok(user(u)),
            Ok(appdata::Response { outcome: Outcome::Error { message, .. }, .. }) => Err(message),
            Ok(_) => Err("the PBX's answer was not understood".to_string()),
            Err(e) => Err(unreachable(&e)),
        };
        let came_online = {
            let mut st = self.state.lock().unwrap();
            match &result {
                Ok(u) => {
                    let was = st.online;
                    st.online = true;
                    st.failed = None;
                    st.user = Some(u.clone());
                    !was
                }
                Err(why) => {
                    st.online = false;
                    st.failed = Some((Instant::now(), why.clone()));
                    false
                }
            }
        };
        match &result {
            Ok(u) if came_online => {
                info!("app data: the PBX's store is in reach; the user is {} ({})", u.name, u.id);
                self.reached.notify_one();
                self.signal(StoreSignal::Online);
            }
            Ok(_) => {}
            Err(why) => debug!("app data: the PBX's store is not in reach: {why}"),
        }
        result.map(|_| ())
    }

    async fn run(&self, request: StoreOp) -> StoreReply {
        let Some(dpma) = self.dpma() else {
            self.offline();
            return StoreReply::Unreachable(DpmaError::NotAttached.to_string());
        };
        let (online, failed) = {
            let st = self.state.lock().unwrap();
            (st.online, st.failed.clone())
        };
        if !online {
            if let Some((when, why)) = failed
                && when.elapsed() < HELLO_RETRY
            {
                return StoreReply::Unreachable(why);
            }
            if let Err(why) = self.hello(&dpma).await {
                return StoreReply::Unreachable(why);
            }
        }
        match dpma.app_data(&op(request)).await {
            Ok(response) => {
                if let Some(u) = response.user {
                    self.state.lock().unwrap().user = Some(user(u));
                }
                reply(response.outcome)
            }
            Err(e) => {
                // The next request, or the next session, says hello again.
                self.offline();
                StoreReply::Unreachable(unreachable(&e))
            }
        }
    }

    /// Follows the DPMA session and the server's events for as long as the
    /// phone runs.
    pub fn follow(&self, store: Store) {
        let this = self.clone();
        self.runtime.spawn(async move {
            let sub = store.subscribe_events(Topic::Provisioning);
            // The server knows the phone by the token of a finished
            // provisioning pass, so that is when to say hello: once for
            // each session, and again, ever more seldom, while the server
            // does not answer (one that keeps no app data never does).
            let mut greeted = false;
            let mut retry = HELLO_RETRY;
            let mut next_try = None;
            loop {
                let (attached, provisioned) = store
                    .read::<Provisioning, _>(|p| (p.attached, p.phase == ProvisionPhase::Provisioned));
                let due = next_try.is_some_and(|at| Instant::now() >= at);
                if !attached {
                    this.offline();
                    greeted = false;
                    retry = HELLO_RETRY;
                    next_try = None;
                } else if provisioned && (!greeted || due) {
                    greeted = true;
                    next_try = None;
                    this.state.lock().unwrap().failed = None;
                    if let Some(dpma) = this.dpma()
                        && this.hello(&dpma).await.is_err()
                    {
                        next_try = Some(Instant::now() + retry);
                        retry = (retry * 2).min(HELLO_RETRY_MAX);
                    }
                }
                let wake = match next_try {
                    Some(at) => {
                        let wait = at.saturating_duration_since(Instant::now());
                        match tokio::time::timeout(wait, sub.changed()).await {
                            Ok(wake) => wake,
                            Err(_) => continue,
                        }
                    }
                    None => sub.changed().await,
                };
                for event in wake.events {
                    if let Event::Push { name, body } = event
                        && name == appdata::EVENT_CHANGE
                        && let Some(change) = appdata::Change::decode(body.as_bytes())
                    {
                        let scope = match change.scope {
                            Scope::Shared => StoreScope::Shared,
                            Scope::User => StoreScope::User,
                        };
                        this.signal(StoreSignal::Changed { app: change.app, scope, key: change.key });
                    }
                }
            }
        });
    }
}

impl PbxStore for AppStore {
    fn request(&self, request: StoreOp, done: Completion<StoreReply>) {
        let this = self.clone();
        self.runtime.spawn(async move { done.complete(this.run(request).await) });
    }

    fn user(&self) -> Option<StoreUser> {
        self.state.lock().unwrap().user.clone()
    }

    fn online(&self) -> bool {
        self.state.lock().unwrap().online && self.dpma().is_some()
    }

    fn watch(&self, apps: &[String]) {
        {
            let mut st = self.state.lock().unwrap();
            if st.watch == apps {
                return;
            }
            st.watch = apps.to_vec();
        }
        // The server has the list of the last hello.
        if let Some(dpma) = self.dpma() {
            let this = self.clone();
            self.runtime.spawn(async move {
                let _ = this.hello(&dpma).await;
            });
        }
    }

    fn subscribe(&self, f: Box<dyn Fn(StoreSignal) + Send + Sync>) {
        self.state.lock().unwrap().subscriber = Some(Arc::from(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_as_the_app_gets_them() {
        let e = Entry { key: "k".into(), value: b"v".to_vec(), version: 3, modified: 9, author_id: "101".into(), author_name: "Alice".into() };
        let StoreReply::Stored(s) = reply(Outcome::Stored(e.clone())) else { panic!() };
        assert_eq!((s.key.as_str(), s.value.as_slice(), s.version, s.modified), ("k", &b"v"[..], 3, 9));
        assert_eq!((s.author_id.as_str(), s.author_name.as_str()), ("101", "Alice"));
        assert_eq!(reply(Outcome::Conflict(None)), StoreReply::Conflict(None));
        assert_eq!(reply(Outcome::Entries { entries: vec![e], more: true }), StoreReply::Entries(vec![s], true));
        assert_eq!(
            reply(Outcome::Error { code: appdata::ErrorCode::Limit, message: "full".into() }),
            StoreReply::Refused("full".into())
        );
        assert!(unreachable(&DpmaError::Timeout).contains("did not answer"));
        assert_eq!(unreachable(&DpmaError::NotAttached), "not attached to a DPMA server");
    }

    #[test]
    fn requests_as_the_server_reads_them() {
        let o = op(StoreOp::Set { app: "game".into(), scope: StoreScope::User, key: "k".into(), value: vec![1], expect: Some(0) });
        assert_eq!(o, Op::Set { app: "game".into(), scope: Scope::User, key: "k".into(), value: vec![1], expect: Some(0) });
        assert_eq!(Op::decode(&o.encode()), Ok(o));
    }
}
