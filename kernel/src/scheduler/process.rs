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
    pub elf_load_addr: *mut u8,
    pub elf_size: usize,

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
    pub next_pid: u32,
}

pub static PROCESS_TABLE: Mutex<ProcessTable> = Mutex::new(ProcessTable {
    table: BTreeMap::new(),
    next_pid: 1,
});

pub fn get_current_pid() -> Option<Pid> {
    let current_tid = super::current();
    let pt = PROCESS_TABLE.lock();
    for (&pid, proc) in &pt.table {
        if proc.main_task == current_tid {
            return Some(pid);
        }
    }
    None
}

pub fn register_process(
    name: &str,
    tid: Tid,
    is_user: bool,
    elf_load_addr: *mut u8,
    elf_size: usize,
) -> Pid {
    let mut pt = PROCESS_TABLE.lock();
    let pid = pt.next_pid;
    pt.next_pid += 1;

    let mut parent_pid = None;
    let mut cwd = String::from("/");
    let current_tid = super::current();
    for (&p, proc) in &pt.table {
        if proc.main_task == current_tid {
            parent_pid = Some(p);
            cwd = proc.cwd.clone();
            break;
        }
    }

    let proc = Process {
        pid,
        parent_pid,
        main_task: tid,
        name: String::from(name),
        state: ProcessState::Running,
        exit_code: 0,
        children: Vec::new(),
        elf_load_addr,
        elf_size,
        cwd,
        pending_signals: 0,
        signal_handlers: [0; 32],
        signal_restorers: [0; 32],
        saved_signal_context: None,
    };

    pt.table.insert(pid, proc);

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
    {
        let mut pt = PROCESS_TABLE.lock();
        if let Some(proc) = pt.table.remove(&pid) {
            if let Some(parent) = proc.parent_pid {
                if let Some(p) = pt.table.get_mut(&parent) {
                    p.children.retain(|c| *c != pid);
                }
            }
        }
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

    let mut current_pid = None;
    for (&pid, proc) in &pt.table {
        if proc.main_task == current_tid {
            current_pid = Some(pid);
            break;
        }
    }

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
