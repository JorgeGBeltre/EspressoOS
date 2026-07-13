#![allow(dead_code)]

use super::task::Tid;
use super::Scheduler;

pub(super) fn next_ready(sched: &mut Scheduler, core: usize) -> Option<Tid> {
    for i in 0..sched.ready.len() {
        let tid = sched.ready[i];
        if let Some(t) = sched.tasks.get(&tid) {
            if t.affinity.is_none() || t.affinity == Some(core) {
                return Some(sched.ready.remove(i));
            }
        }
    }
    None
}

pub fn pick_next(core: usize) -> Option<Tid> {
    super::with_sched(|s| next_ready(s, core)).flatten()
}
