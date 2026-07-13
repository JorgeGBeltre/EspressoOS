//! Abstracción de E/S para la shell: local (consola) o remota (canal SSH).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Hoy `shell::run` está atada a `drivers::uart`. Para servir la MISMA shell por
//! SSH, la E/S se abstrae tras el trait [`ShellIo`]. La adopción completa implica
//! cambiar el bucle de `shell/mod.rs` para leer/escribir a través de un
//! `&mut dyn ShellIo` en vez de llamar directamente a `uart` (ver diseño §6).
#![allow(dead_code)]

use crate::drivers::uart;
use crate::prelude::*;

/// Fuente/destino de bytes de una sesión de shell.
///
/// `read_byte` NO bloquea: devuelve `None` cuando no hay datos ahora mismo, para
/// que el llamador ceda la CPU (`scheduler::yield_now`) igual que hace la shell
/// local. Así el mismo bucle REPL sirve a consola y a un canal SSH.
pub trait ShellIo {
    /// Siguiente byte de entrada, o `None` si no hay ninguno disponible aún.
    fn read_byte(&mut self) -> Option<u8>;
    /// Escribe bytes de salida; devuelve cuántos se aceptaron.
    fn write(&mut self, bytes: &[u8]) -> usize;
}

/// E/S local sobre la consola USB-Serial-JTAG (`drivers::uart`).
pub struct ConsoleIo;

impl ShellIo for ConsoleIo {
    fn read_byte(&mut self) -> Option<u8> {
        uart::getc()
    }
    fn write(&mut self, bytes: &[u8]) -> usize {
        uart::write(bytes)
    }
}

/// E/S remota sobre un canal SSH (`drivers::ssh::channel`). ESQUELETO.
///
/// Enlaza los buffers de entrada/salida del canal: `read_byte` saca del buffer de
/// `CHANNEL_DATA` entrante; `write` encola datos para enviarlos como `CHANNEL_DATA`
/// (respetando la ventana de flujo del canal).
pub struct SshChannelIo {
    // referencia/handle al Channel y a sus colas. TODO(fase-red).
    pub channel_id: u32,
}

impl ShellIo for SshChannelIo {
    fn read_byte(&mut self) -> Option<u8> {
        // TODO(fase-red): pop del buffer de entrada del canal.
        None
    }
    fn write(&mut self, _bytes: &[u8]) -> usize {
        // TODO(fase-red): encolar como CHANNEL_DATA hacia el cliente.
        0
    }
}

/// Variante de la REPL parametrizada por la E/S. ESQUELETO del refactor:
/// `shell::run()` pasaría a ser `run_with_io(&mut ConsoleIo)`, y la sesión SSH
/// llamaría `run_with_io(&mut SshChannelIo{..})`. La lógica de línea/eco/parseo es
/// idéntica a la de `shell::run` actual, sólo cambia el origen/destino de bytes.
pub fn run_with_io(_io: &mut dyn ShellIo) {
    // TODO(fase-red): mover aquí el bucle de shell/mod.rs usando `io` en lugar de
    // las llamadas directas a `uart::getc`/`uart::write`.
}
