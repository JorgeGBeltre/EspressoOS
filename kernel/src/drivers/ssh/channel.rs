#![allow(dead_code)]

use crate::prelude::*;
use crate::scheduler;
use crate::scheduler::process::Pid;
use crate::session::{self, ChannelKind, SessionChannel, SessionConsole};
use crate::vfs;

pub const INITIAL_WINDOW: u32 = 64 * 1024;

pub const MAX_CHANNEL_PACKET: u32 = 32 * 1024;

pub const WINDOW_ADJUST_THRESHOLD: u32 = INITIAL_WINDOW / 2;

const SSH_SHELL_STACK: usize = 8 * 1024;

const SSH_SHELL_PRIO: u8 = 1;

/// The shell behind one SSH channel.
///
/// A session owns its channel, its task, its pid and its fd table -- exactly like
/// the serial console does. No global latch, no shared bridge, so a session
/// cannot inherit a previous one's state.
pub struct SessionShell {
    pub chan: Arc<SessionChannel>,
    pub pid: Pid,
}

impl SessionShell {
    /// Wires up a session and lets it run.
    ///
    /// Blocked on purpose until seeded: one fd operation before `seed_fd_table`
    /// and `or_insert_with(new_process_table)` would hand this task /dev/console,
    /// putting the SSH session's output on the serial port.
    pub fn start(channel_id: u32) -> KResult<SessionShell> {
        let chan = session::create(ChannelKind::Ssh { channel_id });

        let tid = match scheduler::spawn_blocked(
            "ssh-shell",
            ssh_shell_entry,
            chan.id as usize,
            SSH_SHELL_STACK,
            SSH_SHELL_PRIO,
            false,
        ) {
            Ok(t) => t,
            Err(e) => {
                session::destroy(chan.id);
                return Err(e);
            }
        };

        let pid =
            scheduler::process::register_process("ssh-shell", tid, false, None);

        if let Err(e) = vfs::seed_fd_table(pid, SessionConsole::new(chan.clone())) {
            // The task is still blocked and owns nothing, so undoing this is just
            // dropping what we made. It never runs, so it never becomes a zombie
            // for reap_orphans to find -- reap it here.
            scheduler::process::reap(pid);
            session::destroy(chan.id);
            return Err(e);
        }

        scheduler::unblock_task(tid);
        Ok(SessionShell { chan, pid })
    }

    /// Ends the session. The shell's next read sees EOF, `run_session` returns,
    /// and task_trampoline exits the task for us.
    ///
    /// Deliberately does not reap: the task is almost certainly still running and
    /// has not even noticed the EOF yet. Pulling its process entry now would make
    /// get_current_pid() return None underneath it and drop its remaining writes
    /// into pid 0's table. `process::reap_orphans` collects it once it is really
    /// gone.
    pub fn close(&self) {
        session::destroy(self.chan.id);
    }
}

impl Drop for SessionShell {
    fn drop(&mut self) {
        // Covers the paths that never reach an explicit close: a TCP reset that
        // strands the Connection, or the Connection being replaced by the next
        // client. Without this the task would sit forever on a channel nobody can
        // reach.
        self.close();
    }
}

pub struct Channel {
    pub local_id: u32,

    pub remote_id: u32,

    pub send_window: u32,

    pub peer_max_packet: u32,

    pub recv_window: u32,

    pub shell_started: bool,

    pub pty_cols: u32,
    pub pty_rows: u32,

    shell: Option<SessionShell>,
}

impl Channel {
    pub fn new(local_id: u32, remote_id: u32, peer_window: u32, peer_max_packet: u32) -> Self {
        Self {
            local_id,
            remote_id,
            send_window: peer_window,
            peer_max_packet,
            recv_window: INITIAL_WINDOW,
            shell_started: false,
            pty_cols: 80,
            pty_rows: 24,
            shell: None,
        }
    }

    pub fn on_request(&mut self, req_type: &[u8], cols: u32, rows: u32) -> bool {
        match req_type {
            b"pty-req" => {
                self.pty_cols = cols;
                self.pty_rows = rows;
                true
            }
            b"shell" => match SessionShell::start(self.local_id) {
                Ok(s) => {
                    self.shell = Some(s);
                    self.shell_started = true;
                    true
                }
                Err(_) => false,
            },

            _ => false,
        }
    }

    /// True once the shell has exited on its own -- the user typed `exit`, or it
    /// died. Replaces the old "exit requested" flag: the session ends when the
    /// shell actually ends, not when it announces it will.
    ///
    /// Must use `has_exited`, not a Zombie test: reap_orphans runs on this very
    /// task and deletes the entry, so "not Zombie" and "not in the table" are both
    /// "finished" here. Reading a missing pid as still-running would mean we never
    /// send CHANNEL_CLOSE and the client hangs.
    pub fn shell_exited(&self) -> bool {
        match &self.shell {
            Some(s) => scheduler::process::has_exited(s.pid),
            None => false,
        }
    }

    pub fn has_output(&self) -> bool {
        match &self.shell {
            Some(s) => s.chan.has_output(),
            None => false,
        }
    }

    pub fn take_output(&self, max: usize) -> Vec<u8> {
        match &self.shell {
            Some(s) => s.chan.take_output(max),
            None => Vec::new(),
        }
    }

    pub fn close_shell(&self) {
        if let Some(s) = &self.shell {
            s.close();
        }
    }

    pub fn on_data(&mut self, data: &[u8]) -> KResult<Option<u32>> {
        if let Some(s) = &self.shell {
            let _ = s.chan.push_input(data);
        }

        self.recv_window = self.recv_window.saturating_sub(data.len() as u32);
        if self.recv_window < WINDOW_ADJUST_THRESHOLD {
            let add = INITIAL_WINDOW - self.recv_window;
            self.recv_window = self.recv_window.saturating_add(add);
            Ok(Some(add))
        } else {
            Ok(None)
        }
    }

    pub fn add_send_window(&mut self, bytes: u32) {
        self.send_window = self.send_window.saturating_add(bytes);
    }
}

fn ssh_shell_entry(_arg: usize) {
    crate::shell::run_session(Some(crate::drivers::ssh::config::DEV_USER));
    // Returning is the exit path: task_trampoline calls exit(0), which marks the
    // task Zombie for the scheduler and the process Zombie for reap_orphans. The
    // channel reached this task through its fd table, never through `arg`.
}
