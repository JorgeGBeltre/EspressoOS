#![allow(dead_code)]

use alloc::collections::VecDeque;

use crate::arch::xtensa::sync::Mutex;
use crate::drivers::uart;
use crate::prelude::*;
use crate::scheduler;

pub trait ShellIo {

    fn read_byte(&mut self) -> Option<u8>;

    fn write(&mut self, bytes: &[u8]) -> usize;

    fn is_ssh(&self) -> bool {
        false
    }

    fn alive(&self) -> bool {
        true
    }
}

pub struct ConsoleIo;

impl ShellIo for ConsoleIo {
    fn read_byte(&mut self) -> Option<u8> {
        uart::getc()
    }
    fn write(&mut self, bytes: &[u8]) -> usize {
        uart::write(bytes)
    }
}

struct Bridge {

    to_shell: VecDeque<u8>,

    from_shell: VecDeque<u8>,

    open: bool,

    // El shell pidió salir (`exit`). El transporte SSH, tras drenar la salida
    // pendiente, envía CHANNEL_EOF/CLOSE al cliente y cierra la sesión.
    exit_requested: bool,
}

static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);

const BRIDGE_CAP: usize = 16 * 1024;

pub fn bridge_open() {
    let mut g = BRIDGE.lock();
    *g = Some(Bridge {
        to_shell: VecDeque::new(),
        from_shell: VecDeque::new(),
        open: true,
        exit_requested: false,
    });
}

pub fn bridge_close() {
    let mut g = BRIDGE.lock();
    if let Some(b) = g.as_mut() {
        b.open = false;
    }
}

/// El shell solicita salir (`exit`). Marca la bandera y cierra el bridge para
/// que `run_with_io`/`ssh_shell_entry` no reinicien el shell; la salida ya
/// encolada (p.ej. "logout") sigue drenándose porque `bridge_take_output` no
/// mira `open`. El transporte SSH consulta `bridge_exit_requested()` para
/// enviar el CHANNEL_CLOSE al cliente una vez drenada.
pub fn bridge_request_exit() {
    let mut g = BRIDGE.lock();
    if let Some(b) = g.as_mut() {
        b.exit_requested = true;
        b.open = false;
    }
}

pub fn bridge_exit_requested() -> bool {
    BRIDGE.lock().as_ref().map(|b| b.exit_requested).unwrap_or(false)
}

pub fn bridge_clear_exit() {
    if let Some(b) = BRIDGE.lock().as_mut() {
        b.exit_requested = false;
    }
}

pub fn bridge_is_open() -> bool {
    BRIDGE.lock().as_ref().map(|b| b.open).unwrap_or(false)
}

pub fn bridge_push_input(data: &[u8]) -> usize {
    let mut g = BRIDGE.lock();
    if let Some(b) = g.as_mut() {
        let room = BRIDGE_CAP.saturating_sub(b.to_shell.len());
        let n = core::cmp::min(room, data.len());
        b.to_shell.extend(data[..n].iter().copied());
        n
    } else {
        0
    }
}

pub fn bridge_take_output(max: usize) -> Vec<u8> {
    let mut g = BRIDGE.lock();
    if let Some(b) = g.as_mut() {
        let n = core::cmp::min(max, b.from_shell.len());
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(byte) = b.from_shell.pop_front() {
                out.push(byte);
            }
        }
        out
    } else {
        Vec::new()
    }
}

pub fn bridge_has_output() -> bool {
    BRIDGE
        .lock()
        .as_ref()
        .map(|b| !b.from_shell.is_empty())
        .unwrap_or(false)
}

fn bridge_pop_input_byte() -> Option<u8> {
    BRIDGE.lock().as_mut().and_then(|b| b.to_shell.pop_front())
}

fn bridge_write_output(bytes: &[u8]) -> usize {
    let mut g = BRIDGE.lock();
    if let Some(b) = g.as_mut() {
        let room = BRIDGE_CAP.saturating_sub(b.from_shell.len());
        let n = core::cmp::min(room, bytes.len());
        b.from_shell.extend(bytes[..n].iter().copied());
        n
    } else {
        0
    }
}

pub fn command_output_to_ssh(bytes: &[u8]) -> usize {
    bridge_write_output(bytes)
}

pub struct SshChannelIo {

    pub channel_id: u32,
}

impl SshChannelIo {
    pub fn new(channel_id: u32) -> Self {
        Self { channel_id }
    }
}

impl ShellIo for SshChannelIo {
    fn read_byte(&mut self) -> Option<u8> {
        bridge_pop_input_byte()
    }
    fn write(&mut self, bytes: &[u8]) -> usize {
        bridge_write_output(bytes)
    }
    fn is_ssh(&self) -> bool {
        true
    }
    fn alive(&self) -> bool {
        bridge_is_open()
    }
}

pub fn run_with_io(io: &mut dyn ShellIo) {

    io.write(super::banner_bytes());

    let mut line = String::new();
    loop {
        if !io.alive() {
            break;
        }

        // Enruta la salida de los comandos al sink de ESTE shell. Como la consola
        // local y una sesión SSH pueden coexistir, se fija en cada iteración (no una
        // sola vez), para que cada comando escriba donde corresponde.
        if io.is_ssh() {
            crate::shell::commands::set_base_ssh();
        } else {
            crate::shell::commands::set_base_console();
        }

        // Prompt estilo Unix, con el cwd (`/` se muestra como `~`, igual que bash).
        // Por SSH incluye el usuario autenticado (`user@EspressoOS:~$`); en la
        // consola local no hay login, así que se omite (`EspressoOS:~$`).
        let cwd = crate::shell::commands::cwd_get();
        let display_cwd = if cwd == "/" { "~" } else { cwd.as_str() };
        let prompt = if io.is_ssh() {
            alloc::format!(
                "{}@EspressoOS:{}$ ",
                crate::drivers::ssh::config::DEV_USER,
                display_cwd,
            )
        } else {
            alloc::format!("EspressoOS:{}$ ", display_cwd)
        };
        io.write(prompt.as_bytes());
        line.clear();
        if !read_line_io(io, &mut line) {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == "exit" || trimmed == "quit" || trimmed == "logout" {
            io.write(b"logout\r\n");
            if io.is_ssh() {
                // Cierra la sesión SSH: el transporte drena esta última salida y
                // envía CHANNEL_EOF/CLOSE al cliente.
                bridge_request_exit();
            }
            break;
        }

        super::execute_line(trimmed);
    }

    if io.is_ssh() {
        crate::shell::commands::set_base_console();
    }
}

fn read_line_io(io: &mut dyn ShellIo, buf: &mut String) -> bool {
    loop {
        match io.read_byte() {
            Some(byte) => match byte {
                b'\r' | b'\n' => {
                    io.write(b"\r\n");
                    return true;
                }
                0x08 | 0x7f => {
                    if buf.pop().is_some() {
                        io.write(b"\x08 \x08");
                    }
                }
                0x03 => {
                    buf.clear();
                    io.write(b"^C\r\n");
                    return true;
                }
                b if (0x20..0x7f).contains(&b) => {
                    if buf.len() < super::MAX_LINE_LEN {
                        buf.push(b as char);
                        io.write(&[b]);
                    }
                }
                _ => {}
            },
            None => {
                if !io.alive() {
                    return false;
                }

                scheduler::yield_now();
            }
        }
    }
}
