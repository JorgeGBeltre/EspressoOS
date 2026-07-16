#![allow(dead_code)]

use super::task::Tid;
use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use alloc::collections::BTreeMap;

pub type Pid = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Running,
    Zombie,
}

pub struct Process {
    pub pid: Pid,
    pub parent_pid: Option<Pid>,
    pub main_task: Tid,
    pub name: String,
    pub state: ProcessState,
    pub exit_code: i32,
    pub children: Vec<Pid>,

    /// The PSRAM slot this program's image occupies, for user processes.
    ///
    /// Replaces the old `elf_load_addr`/`elf_size` pair: nothing is allocated on
    /// the heap any more, the loader takes a slot out of the reserved region and
    /// whoever reaps the process hands it back. If this is not returned the slot is
    /// gone for the rest of the boot -- there are 32.
    pub slot: Option<crate::mm::psram_exec::SlotIndex>,

    /// Working directory. Per process, not global: two sessions must be able to
    /// `cd` independently, and a child has to inherit its parent's cwd the way it
    /// inherits the fd table.
    pub cwd: String,

    pub pending_signals: u32,
    pub signal_handlers: [usize; 32],
    pub signal_restorers: [usize; 32],
    pub saved_signal_context: Option<esp_hal::xtensa_lx_rt::exception::Context>,
}

unsafe impl Send for Process {}
unsafe impl Sync for Process {}

pub struct ProcessTable {
    pub table: BTreeMap<Pid, Process>,

    /// tid -> pid, the reverse of `Process::main_task`.
    ///
    /// Private, and inside ProcessTable rather than beside it, on purpose. Seven
    /// places used to walk `table` looking for `main_task == current_tid` -- an O(n)
    /// scan under a Mutex that disables interrupts, on the syscall path. A second
    /// `static` map would have fixed the cost and left the two free to drift, with a
    /// lock order to remember. Behind the same guard, mutated by the same methods,
    /// and unnameable from outside this module, they cannot.
    by_tid: BTreeMap<Tid, Pid>,

    pub next_pid: u32,
}

impl ProcessTable {
    /// The pid owning `tid`, if any.
    ///
    /// Takes the tid rather than reading it, so the caller keeps the existing lock
    /// order: `scheduler::current()` locks SCHED, and doing that from in here would
    /// nest PROCESS_TABLE -> SCHED on every syscall.
    pub fn pid_of_tid(&self, tid: Tid) -> Option<Pid> {
        self.by_tid.get(&tid).copied()
    }

    fn insert_process(&mut self, proc: Process) {
        self.by_tid.insert(proc.main_task, proc.pid);
        self.table.insert(proc.pid, proc);
    }

    /// Removes a process and returns it. The only way out of the table: both maps
    /// move together or neither does.
    fn remove_process(&mut self, pid: Pid) -> Option<Process> {
        let proc = self.table.remove(&pid)?;
        self.by_tid.remove(&proc.main_task);
        if let Some(parent) = proc.parent_pid {
            if let Some(p) = self.table.get_mut(&parent) {
                p.children.retain(|c| *c != pid);
            }
        }
        Some(proc)
    }
}

pub static PROCESS_TABLE: Mutex<ProcessTable> = Mutex::new(ProcessTable {
    table: BTreeMap::new(),
    by_tid: BTreeMap::new(),
    next_pid: 1,
});

pub fn get_current_pid() -> Option<Pid> {
    let tid = super::current();
    PROCESS_TABLE.lock().pid_of_tid(tid)
}

/// Detaches an exited child of `parent` and returns (pid, exit code, slot).
///
/// Exists so that sys_wait does not reach into the table itself. It was the one
/// place outside this module that removed a process, which made `by_tid` an
/// invariant maintained by discipline instead of by construction.
pub fn take_zombie_child(parent: Pid) -> Option<(Pid, i32, Option<crate::mm::psram_exec::SlotIndex>)> {
    let mut pt = PROCESS_TABLE.lock();
    let child = pt
        .table
        .get(&parent)?
        .children
        .iter()
        .copied()
        .find(|c| {
            pt.table
                .get(c)
                .map(|p| p.state == ProcessState::Zombie)
                .unwrap_or(false)
        })?;
    let proc = pt.remove_process(child)?;
    Some((proc.pid, proc.exit_code, proc.slot))
}

/// The PSRAM slot of the calling process, if it has one. Kernel tasks do not.
pub fn current_slot() -> Option<crate::mm::psram_exec::SlotIndex> {
    let tid = super::current();
    let pt = PROCESS_TABLE.lock();
    let pid = pt.pid_of_tid(tid)?;
    pt.table.get(&pid)?.slot
}

/// Whether `parent` has any children left to wait for.
pub fn has_children(parent: Pid) -> bool {
    PROCESS_TABLE
        .lock()
        .table
        .get(&parent)
        .map(|p| !p.children.is_empty())
        .unwrap_or(false)
}

pub fn register_process(
    name: &str,
    tid: Tid,
    is_user: bool,
    slot: Option<crate::mm::psram_exec::SlotIndex>,
) -> Pid {
    // Outside the lock: scheduler::current() takes SCHED, and this used to call it
    // while holding PROCESS_TABLE -- nesting the two for no reason.
    let current_tid = super::current();

    let mut pt = PROCESS_TABLE.lock();
    let pid = pt.next_pid;
    pt.next_pid += 1;

    let parent_pid = pt.pid_of_tid(current_tid);
    let cwd = parent_pid
        .and_then(|p| pt.table.get(&p))
        .map(|p| p.cwd.clone())
        .unwrap_or_else(|| String::from("/"));

    let proc = Process {
        pid,
        parent_pid,
        main_task: tid,
        name: String::from(name),
        state: ProcessState::Running,
        exit_code: 0,
        children: Vec::new(),
        slot,
        cwd,
        pending_signals: 0,
        signal_handlers: [0; 32],
        signal_restorers: [0; 32],
        saved_signal_context: None,
    };

    pt.insert_process(proc);

    if let Some(p) = parent_pid {
        if let Some(parent_proc) = pt.table.get_mut(&p) {
            parent_proc.children.push(pid);
        }

        crate::vfs::clone_fd_table(p, pid);
    }

    if is_user {
        super::set_task_user(tid, true);
    }

    pid
}

/// The calling process's working directory, or "/" for a task that has no process
/// (the net and heartbeat tasks). They never resolve paths, so the fallback only
/// exists to keep this total.
pub fn cwd_get() -> String {
    let pid = match get_current_pid() {
        Some(p) => p,
        None => return String::from("/"),
    };
    PROCESS_TABLE
        .lock()
        .table
        .get(&pid)
        .map(|p| p.cwd.clone())
        .unwrap_or_else(|| String::from("/"))
}

/// Sets the calling process's working directory. `path` must already be absolute
/// and normalized.
pub fn cwd_set(path: &str) {
    let pid = match get_current_pid() {
        Some(p) => p,
        None => return,
    };
    if let Some(proc) = PROCESS_TABLE.lock().table.get_mut(&pid) {
        proc.cwd.clear();
        proc.cwd.push_str(path);
    }
}

/// Reports whether `pid` is finished: either sitting Zombie, or already reaped.
///
/// A missing entry means gone, NOT running. Pids are handed out monotonically
/// (`next_pid += 1`) and never reused within a boot, so an absent pid can only be
/// one that has already been cleaned up. Answering "false" there would conflate
/// "still running" with "long gone" -- and since `reap_orphans` is what removes
/// the entry, anything polling for a session to end would poll forever.
pub fn has_exited(pid: Pid) -> bool {
    PROCESS_TABLE
        .lock()
        .table
        .get(&pid)
        .map(|p| p.state == ProcessState::Zombie)
        .unwrap_or(true)
}

/// Drops a process nobody will ever wait() for, releasing its fd table.
///
/// sys_wait only reaps children of a caller that has a pid of its own. Session
/// shells are spawned by the net task, which has none, so their processes get
/// parent_pid = None and would sit Zombie forever -- and the fd table holds the
/// session's Arc<SessionChannel>, so the channel would never be freed either.
/// The spawner reaps them explicitly instead.
///
/// Only call this once the task is actually gone (see `is_zombie`). Removing a
/// live task's process entry would make get_current_pid() return None for it, and
/// its next fd operation would silently fall through to pid 0's table.
pub fn reap(pid: Pid) {
    // Both maps and the parent's child list move together inside remove_process; the
    // guard is released before the cleanup below, which takes other locks.
    let slot = PROCESS_TABLE.lock().remove_process(pid).and_then(|p| p.slot);

    // Give the image's slot back. There are 32, so leaking one per exec would run
    // the system out of them without ever reporting anything.
    if let Some(s) = slot {
        crate::mm::psram_exec::slot_free(s);
    }
    crate::vfs::cleanup_process_fds(pid);
}

/// Reaps every exited process that nobody can wait() for.
///
/// sys_wait only reaps children of a caller that has a pid, so a process with
/// parent_pid = None has no possible waiter -- its entry and its fd table would
/// live forever, and an SSH session's table holds the last Arc to its channel.
/// Rather than chase every path a session can die on (clean logout, client
/// CHANNEL_CLOSE, a TCP reset that strands the Connection, a failure halfway
/// through setup), sweep for the one condition they all end in.
///
/// Safe against a live task only because of the order today's exit path runs in:
/// `scheduler::exit` marks the TASK Zombie before invoking the Exit syscall that
/// marks the PROCESS Zombie, and every exit goes through task_trampoline. A direct
/// sys_exit syscall reverses that -- syscall/handler.rs marks the process Zombie
/// about twenty lines before the task -- so this sweep could reap a process whose
/// task is still running, and its next fd operation would fall through to pid 0's
/// table. Move mark_zombie above the process update there before exposing sys_exit
/// to userland.
pub fn reap_orphans() {
    let dead: Vec<Pid> = {
        let pt = PROCESS_TABLE.lock();
        pt.table
            .iter()
            .filter(|(_, p)| p.state == ProcessState::Zombie && p.parent_pid.is_none())
            .map(|(&pid, _)| pid)
            .collect()
    };
    for pid in dead {
        reap(pid);
    }
}

pub fn check_signals(save_frame: &mut esp_hal::xtensa_lx_rt::exception::Context) -> bool {
    let current_tid = super::current();
    let mut pt = PROCESS_TABLE.lock();

    let current_pid = pt.pid_of_tid(current_tid);

    let pid = match current_pid {
        Some(p) => p,
        None => return false,
    };

    let proc = pt.table.get_mut(&pid).unwrap();
    if proc.pending_signals == 0 {
        return false;
    }

    let mut sig = 0;
    for s in 1..32 {
        if (proc.pending_signals & (1 << s)) != 0 {
            sig = s;
            break;
        }
    }

    if sig == 0 {
        return false;
    }

    proc.pending_signals &= !(1 << sig);

    let handler = proc.signal_handlers[sig];
    let restorer = proc.signal_restorers[sig];

    if handler == 0 {
        if sig == 9 || sig == 2 || sig == 15 {
            drop(pt);
            super::exit(-(sig as i32));
        }
        return false;
    }

    if proc.saved_signal_context.is_some() {
        return false;
    }

    proc.saved_signal_context = Some(*save_frame);

    save_frame.PC = handler as u32;
    save_frame.A2 = sig as u32;
    save_frame.A0 = restorer as u32;

    true
}
