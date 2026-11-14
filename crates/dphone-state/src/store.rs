//! The store: current state, change subscriptions, events, the command
//! bus, heartbeats, and the log ring, behind one cheaply clonable handle.

use crate::command::{Bus, Command, CommandReceiver, Domain, Pending};
use crate::event::Event;
use crate::logring::{Level, LogRing};
use crate::types::*;
use crate::{clock, Model};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

type Callback = Arc<dyn Fn(Topic) + Send + Sync>;

struct Shared {
    state: RwLock<State>,
    subs: Mutex<Vec<Weak<SubShared>>>,
    callbacks: Mutex<Vec<(u64, TopicSet, Callback)>>,
    next_callback: AtomicU64,
    bus: Bus,
    log: LogRing,
    beats: [AtomicU64; Subsystem::ALL.len()],
    next_call: AtomicU32,
    next_notice: AtomicU64,
}

/// Handle to the state store.  Clone it freely; all clones share one store.
///
/// # Reading
/// [`Store::get`] returns one topic (`store.get::<Calls>()`), and
/// [`Store::snapshot`] the whole state.  Both are `Arc` clones: cheap, and
/// immutable once taken.
///
/// # Writing
/// [`Store::update`] runs a closure on a topic and notifies subscribers
/// only if the value actually changed.  Each topic has one owning
/// subsystem (see the type docs); others send [`Command`]s instead of
/// writing.
///
/// # Watching
/// [`Store::subscribe`] gives a [`Subscription`] that can be waited on from
/// a thread ([`Subscription::wait`]) or awaited from any async runtime
/// ([`Subscription::changed`]).  Changes are coalesced per topic: a wake
/// says which topics changed, and the reader takes a fresh snapshot.
/// [`Store::on_change`] registers a callback instead.
#[derive(Clone)]
pub struct Store(Arc<Shared>);

impl Store {
    /// A new store for `model` with default state.
    pub fn new(model: Model) -> Store {
        Store(Arc::new(Shared {
            state: RwLock::new(State::new(model)),
            subs: Mutex::new(Vec::new()),
            callbacks: Mutex::new(Vec::new()),
            next_callback: AtomicU64::new(1),
            bus: Bus::new(),
            log: LogRing::new(2000, 256 * 1024),
            beats: Default::default(),
            next_call: AtomicU32::new(1),
            next_notice: AtomicU64::new(1),
        }))
    }

    /// The model this store was created for.
    pub fn model(&self) -> Model {
        self.0.state.read().unwrap().system.model
    }

    // ------------------------------------------------------------ reads

    /// The whole state.
    pub fn snapshot(&self) -> State {
        self.0.state.read().unwrap().clone()
    }

    /// One topic's state: `let calls = store.get::<Calls>();`
    pub fn get<T: TopicState>(&self) -> Arc<T> {
        T::slot(&self.0.state.read().unwrap()).clone()
    }

    /// Reads one topic through a closure, without cloning the `Arc`.
    pub fn read<T: TopicState, R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(T::slot(&self.0.state.read().unwrap()))
    }

    // ----------------------------------------------------------- writes

    /// Changes one topic.  Subscribers are notified after the closure
    /// returns, and only if the value differs from before.  The closure
    /// runs under the store's write lock: keep it short and do not call
    /// back into the store from inside it.
    pub fn update<T: TopicState, R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let (r, changed) = {
            let mut g = self.0.state.write().unwrap();
            let slot = T::slot_mut(&mut g);
            let old = slot.clone();
            let r = f(Arc::make_mut(slot));
            let changed = *old != **slot;
            if !changed {
                // Keep the old allocation so existing snapshots stay shared.
                *slot = old;
            }
            (r, changed)
        };
        if changed {
            self.notify(T::TOPIC);
        }
        r
    }

    /// Replaces one topic's state.
    pub fn set<T: TopicState>(&self, value: T) {
        self.update::<T, _>(|v| *v = value);
    }

    // ------------------------------------------------------ subscribing

    /// Subscribes to changes of the given topics (no events).
    ///
    /// To avoid missing a change, subscribe first and read the snapshot
    /// second.
    pub fn subscribe(&self, topics: impl Into<TopicSet>) -> Subscription {
        self.subscribe_with(topics, false)
    }

    /// Subscribes to changes of the given topics and to all [`Event`]s.
    pub fn subscribe_events(&self, topics: impl Into<TopicSet>) -> Subscription {
        self.subscribe_with(topics, true)
    }

    fn subscribe_with(&self, topics: impl Into<TopicSet>, events: bool) -> Subscription {
        let shared = Arc::new(SubShared {
            topics: topics.into(),
            events,
            inner: Mutex::new(SubInner {
                dirty: TopicSet::NONE,
                events: VecDeque::new(),
                lost_events: 0,
                waker: None,
            }),
            cv: Condvar::new(),
        });
        self.0.subs.lock().unwrap().push(Arc::downgrade(&shared));
        Subscription { shared, store: self.clone() }
    }

    /// Calls `f` on every change of the given topics, on the thread that
    /// made the change, after the store's lock is released.  `f` must be
    /// quick and must not block; it may read the store.  The callback is
    /// removed when the returned guard is dropped.
    pub fn on_change(
        &self,
        topics: impl Into<TopicSet>,
        f: impl Fn(Topic) + Send + Sync + 'static,
    ) -> CallbackGuard {
        let id = self.0.next_callback.fetch_add(1, Ordering::Relaxed);
        self.0.callbacks.lock().unwrap().push((id, topics.into(), Arc::new(f)));
        CallbackGuard { store: Arc::downgrade(&self.0), id }
    }

    fn notify(&self, topic: Topic) {
        let live: Vec<Arc<SubShared>> = {
            let mut subs = self.0.subs.lock().unwrap();
            subs.retain(|w| w.strong_count() > 0);
            subs.iter().filter_map(Weak::upgrade).filter(|s| s.topics.contains(topic)).collect()
        };
        for s in live {
            let waker = {
                let mut g = s.inner.lock().unwrap();
                g.dirty = g.dirty.with(topic);
                g.waker.take()
            };
            s.cv.notify_all();
            if let Some(w) = waker {
                w.wake();
            }
        }
        let cbs: Vec<Callback> = self
            .0
            .callbacks
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, t, _)| t.contains(topic))
            .map(|(_, _, f)| f.clone())
            .collect();
        for f in cbs {
            f(topic);
        }
    }

    /// Broadcasts a transient event to every event subscription.
    pub fn emit(&self, event: Event) {
        let live: Vec<Arc<SubShared>> = {
            let mut subs = self.0.subs.lock().unwrap();
            subs.retain(|w| w.strong_count() > 0);
            subs.iter().filter_map(Weak::upgrade).filter(|s| s.events).collect()
        };
        for s in live {
            let waker = {
                let mut g = s.inner.lock().unwrap();
                if g.events.len() >= EVENT_CAPACITY {
                    g.events.pop_front();
                    g.lost_events += 1;
                }
                g.events.push_back(event.clone());
                g.waker.take()
            };
            s.cv.notify_all();
            if let Some(w) = waker {
                w.wake();
            }
        }
    }

    // --------------------------------------------------------- commands

    /// Registers the calling subsystem as the handler for a domain,
    /// replacing any earlier handler.
    pub fn handle(&self, domain: Domain) -> CommandReceiver {
        self.0.bus.register(domain)
    }

    /// True when a handler is registered for the domain.
    pub fn has_handler(&self, domain: Domain) -> bool {
        self.0.bus.has_handler(domain)
    }

    /// Sends a command and returns its pending result: call
    /// [`Pending::wait`] from a thread, or `.await` it from a task.
    pub fn send(&self, command: Command) -> Pending {
        self.0.bus.send(command, true)
    }

    /// Sends a command without waiting for the result.  Returns false when
    /// no handler is registered.
    pub fn post(&self, command: Command) -> bool {
        let domain = command.domain();
        if !self.0.bus.has_handler(domain) {
            return false;
        }
        drop(self.0.bus.send(command, false));
        true
    }

    // ----------------------------------------------------------- health

    /// Records that a subsystem's main loop is alive.  Cheap (one atomic
    /// store); call it every loop turn or at least every few seconds.
    pub fn heartbeat(&self, subsystem: Subsystem) {
        // +1 so that 0 means "never".
        self.0.beats[subsystem as usize].store(clock::mono_ms() + 1, Ordering::Relaxed);
    }

    /// When the subsystem last beat, as a monotonic stamp.
    pub fn last_heartbeat(&self, subsystem: Subsystem) -> Option<u64> {
        match self.0.beats[subsystem as usize].load(Ordering::Relaxed) {
            0 => None,
            v => Some(v - 1),
        }
    }

    /// Subsystems that have beaten at least once but not within
    /// `max_age_ms`.
    pub fn stale_subsystems(&self, max_age_ms: u64) -> Vec<Subsystem> {
        let now = clock::mono_ms();
        Subsystem::ALL
            .into_iter()
            .filter(|s| self.last_heartbeat(*s).is_some_and(|t| now.saturating_sub(t) > max_age_ms))
            .collect()
    }

    /// Sets a subsystem's declared status in [`Health`].
    pub fn set_health(&self, subsystem: Subsystem, status: HealthStatus) {
        self.update::<Health, _>(|h| {
            match h.subsystems.iter_mut().find(|e| e.subsystem == subsystem) {
                Some(e) => e.status = status,
                None => h.subsystems.push(SubsystemHealth { subsystem, status }),
            }
        });
    }

    // -------------------------------------------------------------- ids

    /// Allocates a call ID (unique within this boot).
    pub fn next_call_id(&self) -> CallId {
        CallId(self.0.next_call.fetch_add(1, Ordering::Relaxed))
    }

    /// Allocates a notice ID.
    pub fn next_notice_id(&self) -> u64 {
        self.0.next_notice.fetch_add(1, Ordering::Relaxed)
    }

    // -------------------------------------------------------------- log

    /// The log ring.
    pub fn log(&self) -> &LogRing {
        &self.0.log
    }

    /// Appends to the log ring and wakes [`Topic::Log`] subscribers.
    pub fn log_line(&self, level: Level, target: &str, message: String) {
        self.0.log.push(level, target, message);
        self.notify(Topic::Log);
    }

    /// Installs this store as the process-wide `log` backend.  `echo`
    /// also writes each line to standard error (the serial console).
    /// Fails if a logger is already installed.
    pub fn install_logger(
        &self,
        max_level: log::LevelFilter,
        echo: bool,
    ) -> Result<(), log::SetLoggerError> {
        log::set_boxed_logger(Box::new(StoreLogger { store: self.clone(), echo }))?;
        log::set_max_level(max_level);
        Ok(())
    }
}

struct StoreLogger {
    store: Store,
    echo: bool,
}

impl log::Log for StoreLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        let msg = record.args().to_string();
        if self.echo {
            eprintln!("{:5} {}: {}", record.level(), record.target(), msg);
        }
        self.store.log_line(record.level().into(), record.target(), msg);
    }
    fn flush(&self) {}
}

/// Removes an [`Store::on_change`] callback when dropped.  Call
/// [`CallbackGuard::forget`] to keep the callback for the store's life.
pub struct CallbackGuard {
    store: Weak<Shared>,
    id: u64,
}

impl CallbackGuard {
    /// Leaves the callback registered permanently.
    pub fn forget(mut self) {
        self.store = Weak::new();
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        if let Some(s) = self.store.upgrade() {
            s.callbacks.lock().unwrap().retain(|(id, _, _)| *id != self.id);
        }
    }
}

const EVENT_CAPACITY: usize = 256;

struct SubShared {
    topics: TopicSet,
    events: bool,
    inner: Mutex<SubInner>,
    cv: Condvar,
}

struct SubInner {
    dirty: TopicSet,
    events: VecDeque<Event>,
    lost_events: u64,
    waker: Option<Waker>,
}

impl SubInner {
    fn take(&mut self) -> Option<Wake> {
        if self.dirty.is_empty() && self.events.is_empty() {
            return None;
        }
        Some(Wake {
            topics: std::mem::take(&mut self.dirty),
            events: self.events.drain(..).collect(),
            lost_events: std::mem::take(&mut self.lost_events),
        })
    }
}

/// What a [`Subscription`] delivers: the topics that changed since the
/// last wake (coalesced) and the events that arrived, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wake {
    pub topics: TopicSet,
    pub events: Vec<Event>,
    /// Events discarded because the subscriber fell behind.
    pub lost_events: u64,
}

impl Wake {
    pub fn changed(&self, t: Topic) -> bool {
        self.topics.contains(t)
    }
}

/// A subscription to topic changes and (optionally) events.  Dropping it
/// unsubscribes.
pub struct Subscription {
    shared: Arc<SubShared>,
    store: Store,
}

impl Subscription {
    /// The store this subscription belongs to.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Marks every subscribed topic as changed, so the next wait returns
    /// at once.  Handy for "render the initial state through the same
    /// path as updates".
    pub fn mark_all(&self) {
        let waker = {
            let mut g = self.shared.inner.lock().unwrap();
            g.dirty = g.dirty.union(self.shared.topics);
            g.waker.take()
        };
        self.shared.cv.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
    }

    /// Takes pending changes without waiting.
    pub fn try_take(&self) -> Option<Wake> {
        self.shared.inner.lock().unwrap().take()
    }

    /// Blocks until something changes.
    pub fn wait(&self) -> Wake {
        let mut g = self.shared.inner.lock().unwrap();
        loop {
            if let Some(w) = g.take() {
                return w;
            }
            g = self.shared.cv.wait(g).unwrap();
        }
    }

    /// Blocks until something changes or the timeout passes.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Wake> {
        let deadline = Instant::now() + timeout;
        let mut g = self.shared.inner.lock().unwrap();
        loop {
            if let Some(w) = g.take() {
                return Some(w);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            g = self.shared.cv.wait_timeout(g, deadline - now).unwrap().0;
        }
    }

    /// Async: resolves when something changes.  Works on any runtime.
    /// Cancel-safe: nothing is lost if the future is dropped.
    pub fn changed(&self) -> Changed<'_> {
        Changed(self)
    }

    /// Poll form of [`Subscription::changed`], for implementing streams.
    pub fn poll_changed(&self, cx: &mut Context<'_>) -> Poll<Wake> {
        let mut g = self.shared.inner.lock().unwrap();
        match g.take() {
            Some(w) => Poll::Ready(w),
            None => {
                g.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Future returned by [`Subscription::changed`].
pub struct Changed<'a>(&'a Subscription);

impl Future for Changed<'_> {
    type Output = Wake;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Wake> {
        self.0.poll_changed(cx)
    }
}
