//! Abstracción de E/S para la shell: local (consola) o remota (canal SSH).
// COMPILE-STATUS: borrador (implementado, sin compilar contra HW)
//!
//! La MISMA REPL sirve a la consola y a un canal SSH tras el trait [`ShellIo`].
//! `shell::run` es un wrapper sobre [`run_with_io`] con [`ConsoleIo`]; la sesión
//! SSH llama a `run_with_io(&mut SshChannelIo{..})`.
//!
//! PUENTE SSH (`BRIDGE`): la máquina de estados SSH (que vive en `net_task`) y la
//! tarea de la shell remota se comunican por dos colas de bytes globales:
//!   - `to_shell`:   bytes de `CHANNEL_DATA` entrantes -> los lee `SshChannelIo`.
//!   - `from_shell`: salida de la shell -> `net_task` la envía como `CHANNEL_DATA`.
//! Se protegen con el `Mutex` canónico del kernel (enmascara IRQs brevemente). El
//! lock NUNCA se mantiene a través de un `yield_now` (se toma, se copia, se suelta).
//!
//! MVP: UNA sola sesión SSH a la vez (una única instancia de puente).
#![allow(dead_code)]

use alloc::collections::VecDeque;

use crate::arch::xtensa::sync::Mutex;
use crate::drivers::uart;
use crate::prelude::*;
use crate::scheduler;

/// Fuente/destino de bytes de una sesión de shell.
///
/// `read_byte` NO bloquea: devuelve `None` cuando no hay datos, para que el
/// llamador ceda la CPU (`scheduler::yield_now`) igual que la shell local.
pub trait ShellIo {
    /// Siguiente byte de entrada, o `None` si no hay ninguno disponible aún.
    fn read_byte(&mut self) -> Option<u8>;
    /// Escribe bytes de salida; devuelve cuántos se aceptaron.
    fn write(&mut self, bytes: &[u8]) -> usize;
    /// ¿Es una sesión remota (SSH)? La REPL usa esto para enrutar la salida de
    /// los comandos al canal en vez de a la consola. Por defecto, no.
    fn is_ssh(&self) -> bool {
        false
    }
    /// ¿Sigue viva la sesión? La REPL sale del bucle cuando devuelve `false`
    /// (p. ej. el canal SSH se cerró). Por defecto, siempre viva (consola).
    fn alive(&self) -> bool {
        true
    }
}

// ===========================================================================
// E/S local sobre la consola (UART0) — `drivers::uart`.
// ===========================================================================

/// E/S local sobre la consola serie (`drivers::uart`).
pub struct ConsoleIo;

impl ShellIo for ConsoleIo {
    fn read_byte(&mut self) -> Option<u8> {
        uart::getc()
    }
    fn write(&mut self, bytes: &[u8]) -> usize {
        uart::write(bytes)
    }
}

// ===========================================================================
// Puente de bytes entre la máquina de estados SSH y la tarea de la shell.
// ===========================================================================

/// Colas de una sesión SSH activa.
struct Bridge {
    /// Bytes del cliente (CHANNEL_DATA entrante) hacia la shell.
    to_shell: VecDeque<u8>,
    /// Bytes de la shell hacia el cliente (se enviarán como CHANNEL_DATA).
    from_shell: VecDeque<u8>,
    /// La sesión sigue abierta.
    open: bool,
}

/// Puente global (una sesión a la vez en el MVP).
static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);

/// Límite de bytes en cada cola para no crecer sin control (backpressure basto).
const BRIDGE_CAP: usize = 16 * 1024;

/// Abre el puente (lo llama `net_task` cuando arranca la shell del canal).
pub fn bridge_open() {
    let mut g = BRIDGE.lock();
    *g = Some(Bridge {
        to_shell: VecDeque::new(),
        from_shell: VecDeque::new(),
        open: true,
    });
}

/// Cierra el puente (canal/conexión cerrados). La tarea de la shell lo detecta
/// vía `SshChannelIo::alive()` y sale de la REPL.
pub fn bridge_close() {
    let mut g = BRIDGE.lock();
    if let Some(b) = g.as_mut() {
        b.open = false;
    }
}

/// ¿Hay una sesión SSH abierta ahora mismo?
pub fn bridge_is_open() -> bool {
    BRIDGE.lock().as_ref().map(|b| b.open).unwrap_or(false)
}

/// `net_task` empuja aquí los datos de `CHANNEL_DATA` entrantes. Devuelve cuántos
/// bytes se aceptaron (los que exceden `BRIDGE_CAP` se descartan: backpressure).
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

/// `net_task` extrae hasta `max` bytes de salida de la shell para enviarlos como
/// `CHANNEL_DATA`. Devuelve un `Vec` (posiblemente vacío).
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

/// ¿Hay salida pendiente de enviar al cliente?
pub fn bridge_has_output() -> bool {
    BRIDGE
        .lock()
        .as_ref()
        .map(|b| !b.from_shell.is_empty())
        .unwrap_or(false)
}

// -- Acceso interno usado por SshChannelIo. --

/// Saca un byte de entrada (de `to_shell`), o `None`.
fn bridge_pop_input_byte() -> Option<u8> {
    BRIDGE.lock().as_mut().and_then(|b| b.to_shell.pop_front())
}

/// Encola bytes de salida (a `from_shell`). Devuelve cuántos se aceptaron.
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

/// Escribe la salida de un COMANDO al canal SSH (lo usa el sink de `commands`).
pub fn command_output_to_ssh(bytes: &[u8]) -> usize {
    bridge_write_output(bytes)
}

// ===========================================================================
// E/S remota sobre un canal SSH.
// ===========================================================================

/// E/S remota sobre un canal SSH: lee del puente `to_shell`, escribe en
/// `from_shell`. Un `SshChannelIo` sólo tiene sentido con el puente abierto.
pub struct SshChannelIo {
    /// Id local del canal (informativo; el puente es global en el MVP).
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

// ===========================================================================
// REPL genérica sobre `ShellIo` (canónica: la usan consola y SSH).
// ===========================================================================

/// Bucle REPL parametrizado por la E/S. Idéntica lógica de línea/eco/parseo que la
/// shell local; sólo cambia el origen/destino de bytes y el enrutado de la salida
/// de los comandos.
pub fn run_with_io(io: &mut dyn ShellIo) {
    // Si es una sesión SSH, la salida de los comandos (sink global de `commands`)
    // se enruta al canal mientras dure la sesión.
    if io.is_ssh() {
        crate::shell::commands::set_base_ssh();
    }

    io.write(super::banner_bytes());

    let mut line = String::new();
    loop {
        if !io.alive() {
            break;
        }
        io.write(super::prompt_bytes());
        line.clear();
        if !read_line_io(io, &mut line) {
            break; // sesión cerrada mientras leíamos
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // `execute_line` despacha el comando; su salida va al sink global de
        // `commands`, que ya apunta al canal si es sesión SSH.
        super::execute_line(trimmed);
    }

    // Restaurar el sink de la consola al terminar una sesión SSH.
    if io.is_ssh() {
        crate::shell::commands::set_base_console();
    }
}

/// Lee una línea a través de `io` (eco, backspace, Ctrl-C). Devuelve `false` si la
/// sesión se cerró durante la lectura (el llamador debe salir de la REPL).
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
                // Sin datos: cedemos la CPU (cooperativo).
                scheduler::yield_now();
            }
        }
    }
}
