#![allow(dead_code)]

use super::task::Tid;
use super::Scheduler;

pub(super) fn next_ready(sched: &mut Scheduler) -> Option<Tid> {
    if sched.ready.is_empty() {
        None
    } else {

        Some(sched.ready.remove(0))
    }
}

pub fn pick_next() -> Option<Tid> {
    super::with_sched(next_ready).flatten()
}
