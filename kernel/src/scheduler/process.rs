#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;
use alloc::collections::BTreeMap;
use super::task::Tid;

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
    let current_tid = super::current();
    for (&p, proc) in &pt.table {
        if proc.main_task == current_tid {
            parent_pid = Some(p);
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
