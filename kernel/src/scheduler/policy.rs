#![allow(dead_code)]

use super::task::Tid;
use super::Scheduler;

pub(super) fn next_ready(sched: &mut Scheduler, core: usize) -> Option<Tid> {
    let mut best_idx = None;
    let mut highest_prio = 0u8;

    for (i, &tid) in sched.ready.iter().enumerate() {
        if let Some(t) = sched.tasks.get(&tid) {
            if t.affinity.is_none() || t.affinity == Some(core) {
                if best_idx.is_none() || t.priority > highest_prio {
                    highest_prio = t.priority;
                    best_idx = Some(i);
                }
            }
        }
    }

    best_idx.map(|i| sched.ready.remove(i))
}

pub fn pick_next(core: usize) -> Option<Tid> {
    super::with_sched(|s| next_ready(s, core)).flatten()
}
