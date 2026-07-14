#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use crate::prelude::*;
use crate::scheduler;
use crate::shell::remote::{self, SshChannelIo};

pub const INITIAL_WINDOW: u32 = 64 * 1024;

pub const MAX_CHANNEL_PACKET: u32 = 32 * 1024;

pub const WINDOW_ADJUST_THRESHOLD: u32 = INITIAL_WINDOW / 2;

const SSH_SHELL_STACK: usize = 8 * 1024;

static SHELL_TASK_SPAWNED: AtomicBool = AtomicBool::new(false);

pub struct Channel {

    pub local_id: u32,

    pub remote_id: u32,

    pub send_window: u32,

    pub peer_max_packet: u32,

    pub recv_window: u32,

    pub shell_started: bool,

    pub pty_cols: u32,
    pub pty_rows: u32,
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
        }
    }

    pub fn on_request(&mut self, req_type: &[u8], cols: u32, rows: u32) -> bool {
        match req_type {
            b"pty-req" => {
                self.pty_cols = cols;
                self.pty_rows = rows;
                true
            }
            b"shell" => {
                self.start_shell();
                self.shell_started = true;
                true
            }

            _ => false,
        }
    }

    fn start_shell(&self) {
        remote::bridge_open();

        if SHELL_TASK_SPAWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            match scheduler::spawn(
                "ssh-shell",
                ssh_shell_entry,
                self.local_id as usize,
                SSH_SHELL_STACK,
                1,
                false,
            ) {
                Ok(_) => {}
                Err(_) => {

                    SHELL_TASK_SPAWNED.store(false, Ordering::Release);
                }
            }
        }
    }

    pub fn on_data(&mut self, data: &[u8]) -> KResult<Option<u32>> {

        let _ = remote::bridge_push_input(data);

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

fn ssh_shell_entry(arg: usize) {
    let channel_id = arg as u32;
    loop {
        if remote::bridge_is_open() {
            let mut io = SshChannelIo::new(channel_id);
            remote::run_with_io(&mut io);
        }
        scheduler::yield_now();
    }
}
