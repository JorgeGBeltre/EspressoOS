//! Capa de canales SSH: sesión + pty-req + shell (ESQUELETO — RFC 4254).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Tras la autenticación, el cliente abre un canal `session`, pide una `pty-req`
//! y arranca `shell`. Este módulo gestiona la ventana de flujo del canal y
//! puentea `SSH_MSG_CHANNEL_DATA` con la shell a través de `shell::remote`,
//! que expone la misma REPL sobre un stream en vez de sobre la consola.
#![allow(dead_code)]

use crate::prelude::*;

/// Ventana de flujo inicial que anuncia el servidor (bytes).
pub const INITIAL_WINDOW: u32 = 64 * 1024;
/// Tamaño máximo de paquete de datos de canal.
pub const MAX_CHANNEL_PACKET: u32 = 32 * 1024;

/// Estado de un canal de sesión abierto.
pub struct Channel {
    /// Id del canal en el lado del servidor (local).
    pub local_id: u32,
    /// Id del canal en el lado del cliente (remoto).
    pub remote_id: u32,
    /// Ventana disponible para ENVIAR al cliente.
    pub send_window: u32,
    /// Ventana disponible para RECIBIR del cliente.
    pub recv_window: u32,
    /// La sesión ya tiene una shell corriendo.
    pub shell_started: bool,
}

impl Channel {
    /// Procesa una `SSH_MSG_CHANNEL_REQUEST`. Devuelve si se debe responder
    /// SUCCESS. Acepta `pty-req` y `shell`; el resto se rechaza. ESQUELETO.
    pub fn on_request(&mut self, _req_type: &[u8], _want_reply: bool) -> bool {
        // TODO(fase-red): "pty-req" -> guardar term/tamaño; "shell" -> arrancar
        // shell::remote sobre este canal (una tarea del scheduler cuyo ShellIo
        // lee/escribe CHANNEL_DATA). "exec"/"subsystem" -> no soportado.
        false
    }

    /// Entrega datos entrantes del cliente a la shell y consume ventana. ESQUELETO.
    pub fn on_data(&mut self, data: &[u8]) -> KResult<()> {
        // TODO(fase-red): empujar `data` al buffer de entrada de la shell remota;
        // cuando recv_window baje, enviar CHANNEL_WINDOW_ADJUST.
        let _ = data;
        Err(KError::NotSupported)
    }
}

/// Bucle de sesión: mueve bytes entre el canal SSH y la shell remota hasta EOF.
/// ESQUELETO — el motor real vive en la máquina de estados de `super::Connection`.
pub fn session_loop() -> KResult<()> {
    Err(KError::NotSupported)
}
