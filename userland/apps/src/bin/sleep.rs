#![no_std]
#![no_main]

use libc::{println, uptime_ms, yield_now};

/// Holds its slot long enough for a second instance to overlap it.
///
/// Exists to test one thing: that two userland programs can occupy two slots at
/// once. Every other binary finishes instantly, and try_exec blocks, so two
/// sessions could never overlap and the whole point of the slot pool went
/// unexercised -- sequential runs of one program worked even under the old layout
/// where eight of them shared a single fixed slot.
///
/// No sleep syscall: uptime_ms (12) and yield_now (14) already exist and are
/// already exercised, so composing them keeps the new code to four lines of
/// userland. Adding a syscall would mean a failure here could be the concurrency
/// or the new syscall, and the point is to test one thing.
///
/// Busy-yields rather than blocking. That is fine and arguably better: it proves
/// the task is alive and being scheduled while the other session runs, and the
/// scheduler is round-robin so nothing starves.
///
/// Three seconds, fixed. No argv yet.
const SLEEP_MS: usize = 3000;

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("sleep: holding a slot for 3s");
    let start = uptime_ms();
    while uptime_ms().wrapping_sub(start) < SLEEP_MS {
        yield_now();
    }
    println!("sleep: done");
    0
}
