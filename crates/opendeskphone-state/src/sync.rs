//! A small queue usable from both blocking threads and async tasks, with no
//! runtime dependency.  One consumer per queue.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

pub(crate) struct Queue<T> {
    inner: Mutex<Inner<T>>,
    cv: Condvar,
}

struct Inner<T> {
    items: VecDeque<T>,
    waker: Option<Waker>,
    closed: bool,
    cap: usize,
}

impl<T> Queue<T> {
    /// `cap` bounds the queue; when full the oldest item is discarded.
    pub(crate) fn new(cap: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                items: VecDeque::new(),
                waker: None,
                closed: false,
                cap: cap.max(1),
            }),
            cv: Condvar::new(),
        })
    }

    /// Pushes an item.  Returns it back if the queue is closed.
    pub(crate) fn push(&self, item: T) -> Result<(), T> {
        let waker = {
            let mut g = self.inner.lock().unwrap();
            if g.closed {
                return Err(item);
            }
            if g.items.len() >= g.cap {
                g.items.pop_front();
            }
            g.items.push_back(item);
            g.waker.take()
        };
        self.cv.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
        Ok(())
    }

    pub(crate) fn close(&self) {
        let waker = {
            let mut g = self.inner.lock().unwrap();
            g.closed = true;
            g.waker.take()
        };
        self.cv.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    pub(crate) fn try_pop(&self) -> Option<T> {
        self.inner.lock().unwrap().items.pop_front()
    }

    /// Blocks until an item arrives, the queue closes (`None`), or the
    /// timeout passes (`None`).
    pub(crate) fn pop(&self, timeout: Option<Duration>) -> Option<T> {
        let deadline = timeout.map(|t| Instant::now() + t);
        let mut g = self.inner.lock().unwrap();
        loop {
            if let Some(it) = g.items.pop_front() {
                return Some(it);
            }
            if g.closed {
                return None;
            }
            match deadline {
                None => g = self.cv.wait(g).unwrap(),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return None;
                    }
                    g = self.cv.wait_timeout(g, d - now).unwrap().0;
                }
            }
        }
    }

    /// Async form: `Ready(None)` once closed and drained.
    pub(crate) fn poll_pop(&self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut g = self.inner.lock().unwrap();
        if let Some(it) = g.items.pop_front() {
            return Poll::Ready(Some(it));
        }
        if g.closed {
            return Poll::Ready(None);
        }
        g.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Future returned by async receives on a [`Queue`].
pub(crate) struct Pop<'a, T>(pub(crate) &'a Queue<T>);

impl<T> Future for Pop<'_, T> {
    type Output = Option<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.poll_pop(cx)
    }
}
