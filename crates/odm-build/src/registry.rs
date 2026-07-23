//! In-flight build registry: dedups concurrent builds of the same node and
//! turns would-be wait deadlocks into Cycle errors.
//!
//! Per spike-scheduler findings: every wait edge (blocked thread → awaited
//! key → owner thread → its awaited key → ...) is a real dependency edge, so
//! a wait cycle is always a genuine dependency cycle. We check for cycle
//! formation atomically under the lock before blocking, which makes deadlock
//! impossible by construction.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::thread::ThreadId;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RKey {
    /// (context hash, code hash, args hash) — packed into hashes upstream.
    pub context: odm_ir::Hash,
    pub code: odm_ir::Hash,
    pub args: odm_ir::Hash,
}

#[derive(Debug, PartialEq)]
pub enum Acquire {
    /// Caller owns the build; must call `release` when done.
    Owner,
    /// Someone else finished (or failed) the build while we waited; caller
    /// should re-check the memo cache and re-acquire if needed.
    Retry,
    /// Waiting would deadlock: this is a cross-thread dependency cycle.
    Cycle,
    /// The caller's pass was cancelled while waiting.
    Cancelled,
}

#[derive(Default)]
struct Inner {
    in_flight: HashMap<RKey, ThreadId>,
    waiting_on: HashMap<ThreadId, RKey>,
}

#[derive(Default)]
pub struct Registry {
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl Registry {
    /// Try to become the builder for `key`, or wait for the current builder.
    /// `cancelled` is polled while waiting.
    pub fn acquire(&self, key: RKey, cancelled: &dyn Fn() -> bool) -> Acquire {
        let me = std::thread::current().id();
        let mut inner = self.inner.lock().unwrap();
        loop {
            match inner.in_flight.get(&key) {
                None => {
                    inner.in_flight.insert(key, me);
                    return Acquire::Owner;
                }
                Some(&owner) if owner == me => {
                    // Same-thread re-entry is a cycle the chain check should
                    // have caught; treat as cycle defensively.
                    return Acquire::Cycle;
                }
                Some(&owner) => {
                    // Would waiting create a cycle? Walk owner → its awaited
                    // key → that key's owner → ... (bounded by thread count).
                    let mut cur = owner;
                    let cycle = loop {
                        match inner.waiting_on.get(&cur) {
                            None => break false,
                            Some(k) => match inner.in_flight.get(k) {
                                Some(&next) if next == me => break true,
                                Some(&next) => cur = next,
                                None => break false,
                            },
                        }
                    };
                    if cycle {
                        return Acquire::Cycle;
                    }
                    inner.waiting_on.insert(me, key);
                    let (guard, _timeout) =
                        self.cv.wait_timeout(inner, Duration::from_millis(50)).unwrap();
                    inner = guard;
                    inner.waiting_on.remove(&me);
                    if cancelled() {
                        return Acquire::Cancelled;
                    }
                    if !inner.in_flight.contains_key(&key) {
                        return Acquire::Retry;
                    }
                    // Owner still running (or a new owner took over): loop.
                }
            }
        }
    }

    pub fn release(&self, key: RKey) {
        let mut inner = self.inner.lock().unwrap();
        inner.in_flight.remove(&key);
        drop(inner);
        self.cv.notify_all();
    }
}
