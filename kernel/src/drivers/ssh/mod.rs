//! Servidor SSH-2.0 (ESQUELETO — servicio de red, gating por Fase 7).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Acepta conexiones TCP en el puerto 22 y conduce la máquina de estados de la
//! conexión SSH (ver `docs/ssh-design.md` §4). La criptografía se delega a
//! `kex`/`crypt`/`auth` (que a su vez llaman a crates auditadas). La E/S de la
//! shell remota se puentea vía `shell::remote`.
//!
//! PRERREQUISITO (no funciona sin esto): la red (`drivers::wifi` + `smoltcp`)
//! debe estar cableada y ofrecer un socket TCP en modo ESCUCHA (hoy `wifi.rs`
//! sólo tiene cliente TCP). Ver `docs/ssh-design.md` §2.
#![allow(dead_code)]

pub mod auth;
pub mod channel;
pub mod crypt;
pub mod kex;
pub mod proto;

use crate::prelude::*;

/// Puerto TCP estándar de SSH.
pub const SSH_PORT: u16 = 22;

/// Transporte de bytes fiable subyacente (un socket TCP de smoltcp).
///
/// Se abstrae para no acoplar el protocolo al backend de red concreto y para
/// poder probar la máquina de estados sobre un transporte simulado.
pub trait Transport {
    /// Lee hasta llenar `buf` o hasta que no haya más datos ahora.
    fn read(&mut self, buf: &mut [u8]) -> KResult<usize>;
    /// Escribe `buf`; devuelve cuántos bytes aceptó.
    fn write(&mut self, buf: &[u8]) -> KResult<usize>;
    /// Cierra el envío (half-close).
    fn close(&mut self);
}

/// Fase de la conexión SSH (ver máquina de estados del diseño).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    VersionExchange,
    KexInit,
    Kex,
    Encrypted, // NEWKEYS intercambiado
    UserAuth,
    Session,
    Closed,
}

/// Clave de host persistente del servidor (ssh-ed25519).
///
/// Debe generarse una vez (con el TRNG) y guardarse en el FS (LittleFS, Fase 4);
/// su huella la verifica el cliente en `known_hosts`. Hoy es un marcador.
pub struct HostKey {
    // ed25519_dalek::SigningKey (32 bytes de semilla). TODO(fase-red).
    pub present: bool,
}

/// Estado de una conexión SSH en curso.
pub struct Connection {
    pub state: State,
    // session_id: [u8;32] (el H del primer kex), claves de cifrado, contadores de
    // secuencia, ventana del canal, etc. Se rellenan según avanza el handshake.
}

impl Connection {
    pub fn new() -> Self {
        Self { state: State::VersionExchange }
    }

    /// Conduce una conexión hasta su cierre. ESQUELETO del bucle principal.
    ///
    /// El flujo real (resumen): intercambiar versiones -> KEXINIT -> `kex::run`
    /// (X25519 + firma ed25519 del hash H) -> NEWKEYS -> `crypt` activo ->
    /// SERVICE_REQUEST(ssh-userauth) -> `auth::authenticate` -> CHANNEL_OPEN/REQUEST
    /// -> `channel::session_loop` (puentea con `shell::remote`).
    pub fn run<T: Transport>(&mut self, _t: &mut T, _host: &HostKey) -> KResult<()> {
        // TODO(fase-red): implementar la máquina de estados descrita arriba.
        // Cada transición usa proto::{frame_packet,parse_packet} + los tipos
        // RFC 4251 (ya probados) y, tras NEWKEYS, crypt::Aead para cifrar/descifrar.
        Err(KError::NotSupported)
    }
}

/// Arranca el servidor SSH: bind al puerto 22 y atiende conexiones. ESQUELETO.
///
/// Requiere el socket TCP de escucha del stack de red (pendiente en `drivers::wifi`).
pub fn serve() -> KResult<()> {
    // TODO(fase-red): obtener un socket TCP escuchando en SSH_PORT desde la capa
    // de red; por cada accept, crear una Connection y llamar a run().
    Err(KError::NotSupported)
}
