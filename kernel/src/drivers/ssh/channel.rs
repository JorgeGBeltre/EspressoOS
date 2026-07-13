//! Capa de canales SSH: sesión + pty-req + shell (RFC 4254).
// COMPILE-STATUS: borrador (implementado, sin compilar contra HW)
//!
//! Tras la autenticación, el cliente abre un canal `session`, pide una `pty-req` y
//! arranca `shell`. Este módulo gestiona el estado del canal y la ventana de flujo;
//! el puente real de bytes con la shell lo hace `shell::remote` (colas globales), y
//! el movimiento de `CHANNEL_DATA` lo conduce la máquina de estados de
//! `super::Connection`.
#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use crate::prelude::*;
use crate::scheduler;
use crate::shell::remote::{self, SshChannelIo};

/// Ventana de flujo inicial que anuncia el servidor (bytes).
pub const INITIAL_WINDOW: u32 = 64 * 1024;
/// Tamaño máximo de paquete de datos de canal.
pub const MAX_CHANNEL_PACKET: u32 = 32 * 1024;
/// Umbral: cuando `recv_window` baje de esto, enviar CHANNEL_WINDOW_ADJUST.
pub const WINDOW_ADJUST_THRESHOLD: u32 = INITIAL_WINDOW / 2;
/// Pila de la tarea de la shell remota.
const SSH_SHELL_STACK: usize = 8 * 1024;

/// Sólo se arranca UNA tarea de shell remota en todo el sistema (MVP: una sesión
/// a la vez). La tarea corre en bucle y sirve la sesión mientras el puente esté
/// abierto; en reconexiones se reutiliza.
static SHELL_TASK_SPAWNED: AtomicBool = AtomicBool::new(false);

/// Estado de un canal de sesión abierto.
pub struct Channel {
    /// Id del canal en el lado del servidor (local).
    pub local_id: u32,
    /// Id del canal en el lado del cliente (remoto).
    pub remote_id: u32,
    /// Ventana disponible para ENVIAR al cliente.
    pub send_window: u32,
    /// Tamaño máximo de paquete que acepta el cliente.
    pub peer_max_packet: u32,
    /// Ventana disponible para RECIBIR del cliente.
    pub recv_window: u32,
    /// La sesión ya tiene una shell corriendo.
    pub shell_started: bool,
    /// Dimensiones del pty (si se pidió pty-req).
    pub pty_cols: u32,
    pub pty_rows: u32,
}

impl Channel {
    /// Crea el canal local en respuesta a un CHANNEL_OPEN del cliente.
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

    /// Procesa una `SSH_MSG_CHANNEL_REQUEST`. `req_type` y el resto de campos ya
    /// parseados los aporta `Connection`. Devuelve si se debe responder
    /// `CHANNEL_SUCCESS` (`true`) o `CHANNEL_FAILURE` (`false`).
    ///
    /// - `pty-req`: guarda dimensiones y acepta.
    /// - `shell`: arranca la shell remota (una tarea) y acepta.
    /// - resto (`exec`, `subsystem`, ...): rechaza.
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
            // exec/subsystem fuera de alcance del MVP.
            _ => false,
        }
    }

    /// Arranca la shell remota: abre el puente y (si aún no existe) lanza la tarea
    /// que corre la REPL sobre `SshChannelIo`.
    fn start_shell(&self) {
        remote::bridge_open();
        // Spawnear la tarea de shell SÓLO la primera vez; luego se reutiliza (su
        // bucle externo espera a que el puente esté abierto).
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
            ) {
                Ok(_) => {}
                Err(_) => {
                    // No se pudo crear la tarea: permitir reintento futuro.
                    SHELL_TASK_SPAWNED.store(false, Ordering::Release);
                }
            }
        }
    }

    /// Entrega datos entrantes del cliente a la shell y consume ventana de RECIBIR.
    ///
    /// Devuelve `Some(bytes_a_añadir)` si `recv_window` bajó del umbral y hay que
    /// enviar un `CHANNEL_WINDOW_ADJUST` para reponerla; `None` en otro caso.
    pub fn on_data(&mut self, data: &[u8]) -> KResult<Option<u32>> {
        // Empujar al puente hacia la shell.
        let _ = remote::bridge_push_input(data);
        // Descontar de la ventana de recepción.
        self.recv_window = self.recv_window.saturating_sub(data.len() as u32);
        if self.recv_window < WINDOW_ADJUST_THRESHOLD {
            let add = INITIAL_WINDOW - self.recv_window;
            self.recv_window = self.recv_window.saturating_add(add);
            Ok(Some(add))
        } else {
            Ok(None)
        }
    }

    /// Añade crédito a la ventana de ENVÍO (CHANNEL_WINDOW_ADJUST del cliente).
    pub fn add_send_window(&mut self, bytes: u32) {
        self.send_window = self.send_window.saturating_add(bytes);
    }
}

/// Cuerpo de la tarea de la shell remota. Bucle externo: mientras haya una sesión
/// SSH con el puente abierto, sirve la REPL; si no, cede la CPU. Nunca panica; al
/// cerrarse el puente, `run_with_io` retorna y esperamos la siguiente sesión.
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
