//! Servidor SSH-2.0: listener + máquina de estados de la conexión.
// COMPILE-STATUS: borrador (implementado, sin compilar contra HW)
//!
//! Conduce la conexión SSH (ver `docs/ssh-design.md` §4):
//!   VersionExchange -> KexInit -> Kex -> Encrypted -> UserAuth -> Session.
//! La criptografía se delega a `kex`/`crypt`/`auth` (crates auditadas). La E/S de
//! la shell remota se puentea vía `shell::remote` (colas globales).
//!
//! DECISIÓN DE INTEGRACIÓN: la máquina de estados se BOMBEA de forma no bloqueante
//! desde `net_task` (`drivers::wifi`), sobre un `Transport` (socket TCP de smoltcp).
//! Cada `Connection::pump` hace TODO el trabajo disponible sin bloquear y regresa,
//! para que `net_task` siga atendiendo el resto del stack (mismo criterio que el
//! servidor de eco). Una sola conexión SSH a la vez basta para el MVP.
#![allow(dead_code)]

pub mod auth;
pub mod channel;
pub mod config;
pub mod crypt;
pub mod crypto_rng; // adaptador TRNG -> rand_core 0.6 (fuente de entropía)
pub mod crypto_smoke; // PASO 1 de-riesgo: ejercita/enlaza las crates de cripto
pub mod kex;
pub mod proto;

use core::str;

use esp_println::println;
use rand_core::RngCore;
use sha2::{Digest, Sha256};

use crate::prelude::*;
use crate::shell::remote;

use channel::Channel;
use crypt::Aead;
use crypto_rng::HwRng;
use ed25519_dalek::SigningKey;
use proto::{frame_packet, Reader, Writer};

/// Puerto TCP estándar de SSH.
pub const SSH_PORT: u16 = 22;

/// Mensaje adicional no declarado en `proto` (RFC 4252 §7): PK_OK del sondeo.
const SSH_MSG_USERAUTH_PK_OK: u8 = 60;

/// Máximo de intentos de autenticación antes de cerrar la conexión.
const MAX_AUTH_ATTEMPTS: u32 = 6;

/// Razones de DISCONNECT (RFC 4253 §11.1) que usamos.
const DISCONNECT_KEY_EXCHANGE_FAILED: u32 = 3;
const DISCONNECT_PROTOCOL_ERROR: u32 = 2;
const DISCONNECT_BY_APPLICATION: u32 = 11;

/// Transporte de bytes fiable subyacente (un socket TCP de smoltcp).
pub trait Transport {
    /// Lee hasta llenar `buf` o hasta que no haya más datos ahora (`Ok(0)`).
    fn read(&mut self, buf: &mut [u8]) -> KResult<usize>;
    /// Escribe `buf`; devuelve cuántos bytes aceptó.
    fn write(&mut self, buf: &[u8]) -> KResult<usize>;
    /// Cierra el envío (half-close).
    fn close(&mut self);
}

/// Fase de la conexión SSH.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    VersionExchange,
    KexInit,
    Kex,
    Encrypted, // NEWKEYS intercambiado; esperando SERVICE_REQUEST
    UserAuth,
    Session,
    Closed,
}

/// Clave de host persistente del servidor (ssh-ed25519).
///
/// En el MVP se genera al arranque con el TRNG (no se persiste todavía; cuando
/// esté LittleFS se guardará la semilla de 32 bytes y se recargará). Su huella se
/// imprime por consola para que el operador la fije en `known_hosts`.
pub struct HostKey {
    signing: SigningKey,
}

impl HostKey {
    /// Genera una clave de host nueva con el TRNG (radio activa: `net_task`).
    pub fn generate(rng: &mut HwRng) -> Self {
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed);
        let signing = SigningKey::from_bytes(&seed);
        Self { signing }
    }

    /// Carga una clave de host desde una semilla persistida (32 bytes).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    fn signing(&self) -> &SigningKey {
        &self.signing
    }

    /// Blob público `K_S` (`string "ssh-ed25519" || string pub(32)`).
    pub fn public_blob(&self) -> Vec<u8> {
        kex::host_key_blob(&self.signing.verifying_key())
    }

    /// Huella estilo openssh: `SHA256:<base64(sha256(K_S)) sin padding>`.
    pub fn fingerprint(&self) -> String {
        let blob = self.public_blob();
        let digest: [u8; 32] = Sha256::digest(&blob).into();
        let mut s = String::from("SHA256:");
        s.push_str(&base64_nopad(&digest));
        s
    }
}

/// Codifica en base64 estándar SIN padding (huella openssh).
fn base64_nopad(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut chunks = data.chunks(3);
    for c in &mut chunks {
        let b0 = c[0] as u32;
        let b1 = *c.get(1).unwrap_or(&0) as u32;
        let b2 = *c.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if c.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if c.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

/// Estado de una conexión SSH en curso, bombeada por `net_task`.
pub struct Connection {
    state: State,
    started: bool,

    // Buffers de E/S sin bloqueo.
    rx: Vec<u8>,
    tx: Vec<u8>,

    // Transcripción para el hash de intercambio.
    v_c: Vec<u8>, // ident del cliente (sin CRLF)
    i_c: Vec<u8>, // payload del KEXINIT del cliente
    i_s: Vec<u8>, // payload del KEXINIT del servidor

    // Resultado del kex.
    session_id: Option<[u8; 32]>,

    // Contadores de secuencia GLOBALES (NO se reinician con NEWKEYS).
    send_seq: u32,
    recv_seq: u32,

    // Estado de cifrado por sentido.
    out_encrypted: bool,
    in_encrypted: bool,
    enc_out: Option<Aead>, // s2c (seal)
    enc_in: Option<Aead>,  // c2s (open)
    awaiting_client_newkeys: bool,

    // Autenticación.
    authed: bool,
    auth_user: String,
    auth_fails: u32,

    // Canal de sesión (uno solo en el MVP).
    channel: Option<Channel>,

    // Fuente de entropía (para padding y el par efímero del kex).
    rng: HwRng,
}

impl Connection {
    pub fn new(rng: HwRng) -> Self {
        Self {
            state: State::VersionExchange,
            started: false,
            rx: Vec::new(),
            tx: Vec::new(),
            v_c: Vec::new(),
            i_c: Vec::new(),
            i_s: Vec::new(),
            session_id: None,
            send_seq: 0,
            recv_seq: 0,
            out_encrypted: false,
            in_encrypted: false,
            enc_out: None,
            enc_in: None,
            awaiting_client_newkeys: false,
            authed: false,
            auth_user: String::new(),
            auth_fails: 0,
            channel: None,
            rng,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn is_closed(&self) -> bool {
        self.state == State::Closed
    }

    // -----------------------------------------------------------------------
    // Bucle de bombeo (no bloqueante). Lo llama `net_task` en cada pasada.
    // -----------------------------------------------------------------------

    pub fn pump<T: Transport>(&mut self, t: &mut T, host: &HostKey) -> KResult<()> {
        if self.state == State::Closed {
            return Ok(());
        }

        // 0. Saludo inicial: ident + KEXINIT del servidor (una sola vez).
        if !self.started {
            self.started = true;
            // El ident NO es un paquete binario (no cuenta para `send_seq`).
            self.tx.extend_from_slice(proto::IDENT.as_bytes());
            self.tx.extend_from_slice(b"\r\n");
            let kexinit = self.build_server_kexinit();
            self.i_s = kexinit.clone();
            self.send_packet(&kexinit)?; // primer paquete binario -> seq 0
        }

        // 1. Drenar el transporte hacia `rx`.
        self.read_all(t)?;

        // 2. Intercambio de versión (líneas de texto, no paquetes).
        if self.state == State::VersionExchange {
            match self.try_take_version_line()? {
                Some(vc) => {
                    self.v_c = vc;
                    self.state = State::KexInit;
                }
                None => {
                    self.flush(t)?;
                    return Ok(()); // faltan bytes de la línea de versión
                }
            }
        }

        // 3. Procesar todos los paquetes disponibles.
        while self.state != State::Closed {
            let payload = match self.next_packet()? {
                Some(p) => p,
                None => break,
            };
            self.handle_packet(&payload, host)?;
        }

        // 4. Empujar la salida de la shell hacia el cliente.
        if self.state == State::Session {
            self.pump_shell_output()?;
        }

        // 5. Volcar `tx` al transporte.
        self.flush(t)?;
        Ok(())
    }

    /// Lee todo lo disponible del transporte a `rx` (sin bloquear).
    fn read_all<T: Transport>(&mut self, t: &mut T) -> KResult<()> {
        let mut tmp = [0u8; 1024];
        loop {
            match t.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    self.rx.extend_from_slice(&tmp[..n]);
                    if n < tmp.len() {
                        break;
                    }
                }
                Err(KError::WouldBlock) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Vuelca `tx` al transporte, conservando lo que el socket no acepte aún.
    fn flush<T: Transport>(&mut self, t: &mut T) -> KResult<()> {
        if self.tx.is_empty() {
            return Ok(());
        }
        let mut sent = 0;
        while sent < self.tx.len() {
            match t.write(&self.tx[sent..]) {
                Ok(0) => break,
                Ok(n) => sent += n,
                Err(KError::WouldBlock) => break,
                Err(e) => return Err(e),
            }
        }
        if sent > 0 {
            self.tx.drain(0..sent);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Envío/recepción de paquetes (framing + AEAD).
    // -----------------------------------------------------------------------

    /// Enmarca `payload`, lo cifra si procede y lo encola en `tx`. Incrementa el
    /// contador de secuencia de envío SIEMPRE (cuente o no como cifrado).
    fn send_packet(&mut self, payload: &[u8]) -> KResult<()> {
        // Padding: `frame_packet` sólo admite un byte de relleno. Usamos un byte
        // aleatorio del TRNG. LIMITACIÓN: el padding ideal es TODO aleatorio; para
        // los paquetes cifrados el padding va cifrado (irrelevante), y para los
        // pocos paquetes en claro (KEXINIT/…) es una desviación menor de RFC 4253.
        let pad_fill = (self.rng.next_u32() & 0xff) as u8;
        let framed = frame_packet(payload, proto::MIN_BLOCK, pad_fill);
        let seq = self.send_seq;
        let record = if self.out_encrypted {
            let aead = self.enc_out.clone().ok_or(KError::InvalidArgument)?;
            aead.seal(&framed, seq)?
        } else {
            framed
        };
        self.tx.extend_from_slice(&record);
        self.send_seq = self.send_seq.wrapping_add(1);
        Ok(())
    }

    /// Extrae el siguiente paquete disponible de `rx` (descifrando si procede).
    /// `Ok(None)` si aún no hay un paquete completo. Incrementa `recv_seq`.
    fn next_packet(&mut self) -> KResult<Option<Vec<u8>>> {
        if self.in_encrypted {
            let aead = self.enc_in.clone().ok_or(KError::InvalidArgument)?;
            if self.rx.len() < 4 {
                return Ok(None);
            }
            let mut enc_len = [0u8; 4];
            enc_len.copy_from_slice(&self.rx[..4]);
            let len = aead.open_length(&enc_len, self.recv_seq)? as usize;
            if len < 1 + proto::MIN_PADDING || len > proto::MAX_PACKET {
                return Err(KError::InvalidArgument);
            }
            let total = 4 + len + crypt::TAG_LEN;
            if self.rx.len() < total {
                return Ok(None);
            }
            let record = self.rx[..total].to_vec();
            let payload = aead.open(&record, self.recv_seq)?;
            self.rx.drain(0..total);
            self.recv_seq = self.recv_seq.wrapping_add(1);
            Ok(Some(payload))
        } else {
            if self.rx.len() < 4 {
                return Ok(None);
            }
            let plen =
                u32::from_be_bytes([self.rx[0], self.rx[1], self.rx[2], self.rx[3]]) as usize;
            if plen < 1 + proto::MIN_PADDING || plen > proto::MAX_PACKET {
                return Err(KError::InvalidArgument);
            }
            let total = 4 + plen;
            if self.rx.len() < total {
                return Ok(None);
            }
            let (payload, consumed) = proto::parse_packet(&self.rx)?;
            self.rx.drain(0..consumed);
            self.recv_seq = self.recv_seq.wrapping_add(1);
            Ok(Some(payload))
        }
    }

    // -----------------------------------------------------------------------
    // Intercambio de versión.
    // -----------------------------------------------------------------------

    /// Busca en `rx` una línea `SSH-2.0-…` terminada en `\n`. Descarta líneas de
    /// banner previas. Devuelve el ident del cliente (sin CRLF) o `None` si aún no
    /// hay una línea completa.
    fn try_take_version_line(&mut self) -> KResult<Option<Vec<u8>>> {
        loop {
            let nl = match self.rx.iter().position(|&b| b == b'\n') {
                Some(i) => i,
                None => {
                    if self.rx.len() > 255 {
                        return Err(KError::InvalidArgument); // línea desmesurada
                    }
                    return Ok(None);
                }
            };
            // Extraer la línea (sin el \n) y quitar un \r final.
            let mut line: Vec<u8> = self.rx[..nl].to_vec();
            self.rx.drain(0..=nl);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.len() > 255 {
                return Err(KError::InvalidArgument);
            }
            if line.starts_with(b"SSH-") {
                return Ok(Some(line));
            }
            // Línea de banner anterior al ident: se descarta y se sigue buscando.
        }
    }

    // -----------------------------------------------------------------------
    // Construcción y negociación del KEXINIT.
    // -----------------------------------------------------------------------

    fn build_server_kexinit(&mut self) -> Vec<u8> {
        let mut cookie = [0u8; 16];
        self.rng.fill_bytes(&mut cookie);
        let mut w = Writer::new();
        w.put_u8(proto::SSH_MSG_KEXINIT);
        // cookie (16 bytes crudos).
        for b in cookie.iter() {
            w.put_u8(*b);
        }
        w.put_name_list(&[kex::KEX_NAME, kex::KEX_NAME_ALIAS]) // kex_algorithms
            .put_name_list(&[kex::HOSTKEY_NAME]) // server_host_key_algorithms
            .put_name_list(&[crypt::CIPHER_NAME]) // enc c2s
            .put_name_list(&[crypt::CIPHER_NAME]) // enc s2c
            .put_name_list(&[]) // mac c2s (AEAD -> vacío)
            .put_name_list(&[]) // mac s2c
            .put_name_list(&["none"]) // comp c2s
            .put_name_list(&["none"]) // comp s2c
            .put_name_list(&[]) // languages c2s
            .put_name_list(&[]) // languages s2c
            .put_bool(false) // first_kex_packet_follows
            .put_u32(0); // reserved
        w.into_bytes()
    }

    /// Comprueba que el KEXINIT del cliente ofrece los algoritmos que exigimos.
    /// Devuelve `Err` si falta alguno esencial (kex o cifrado).
    fn negotiate(&self, client_kexinit: &[u8]) -> KResult<()> {
        let mut r = Reader::new(client_kexinit);
        let _msg = r.get_u8()?; // 20
        let _cookie = {
            // saltar 16 bytes de cookie
            for _ in 0..16 {
                let _ = r.get_u8()?;
            }
        };
        let kex_algs = r.get_name_list()?;
        let _host_algs = r.get_name_list()?;
        let enc_c2s = r.get_name_list()?;
        let enc_s2c = r.get_name_list()?;
        // El resto no lo necesitamos para la comprobación mínima.

        let kex_ok = kex_algs
            .iter()
            .any(|a| a == kex::KEX_NAME || a == kex::KEX_NAME_ALIAS);
        let enc_ok = enc_c2s.iter().any(|a| a == crypt::CIPHER_NAME)
            && enc_s2c.iter().any(|a| a == crypt::CIPHER_NAME);
        if kex_ok && enc_ok {
            Ok(())
        } else {
            Err(KError::NotSupported)
        }
    }

    // -----------------------------------------------------------------------
    // Despacho de mensajes por estado.
    // -----------------------------------------------------------------------

    fn handle_packet(&mut self, payload: &[u8], host: &HostKey) -> KResult<()> {
        if payload.is_empty() {
            return Ok(());
        }
        let msg = payload[0];

        // Mensajes globales aceptados en cualquier estado.
        match msg {
            proto::SSH_MSG_DISCONNECT => {
                self.state = State::Closed;
                return Ok(());
            }
            proto::SSH_MSG_IGNORE | proto::SSH_MSG_DEBUG => return Ok(()),
            proto::SSH_MSG_GLOBAL_REQUEST => {
                // want_reply está tras un `string`; respondemos FAILURE si aplica.
                let mut r = Reader::new(payload);
                let _ = r.get_u8();
                let _name = r.get_string();
                if let Ok(want_reply) = r.get_bool() {
                    if want_reply {
                        let mut w = Writer::new();
                        w.put_u8(proto::SSH_MSG_REQUEST_FAILURE);
                        let p = w.into_bytes();
                        self.send_packet(&p)?;
                    }
                }
                return Ok(());
            }
            _ => {}
        }

        match self.state {
            State::KexInit => self.on_kexinit(payload, msg),
            State::Kex => self.on_kex(payload, msg, host),
            State::Encrypted => self.on_service_request(payload, msg),
            State::UserAuth => self.on_userauth(payload, msg),
            State::Session => self.on_session(payload, msg),
            _ => Ok(()),
        }
    }

    fn on_kexinit(&mut self, payload: &[u8], msg: u8) -> KResult<()> {
        if msg != proto::SSH_MSG_KEXINIT {
            return self.disconnect(DISCONNECT_PROTOCOL_ERROR, "se esperaba KEXINIT");
        }
        // Guardar I_C (payload íntegro) y negociar.
        self.i_c = payload.to_vec();
        if self.negotiate(payload).is_err() {
            return self.disconnect(DISCONNECT_KEY_EXCHANGE_FAILED, "sin algoritmos comunes");
        }
        self.state = State::Kex;
        Ok(())
    }

    fn on_kex(&mut self, payload: &[u8], msg: u8, host: &HostKey) -> KResult<()> {
        // Puede llegar KEX_ECDH_INIT (30) o, después, el NEWKEYS del cliente (21).
        if msg == proto::SSH_MSG_NEWKEYS {
            if self.awaiting_client_newkeys {
                self.in_encrypted = true;
                self.awaiting_client_newkeys = false;
                self.state = State::Encrypted;
                return Ok(());
            }
            return self.disconnect(DISCONNECT_PROTOCOL_ERROR, "NEWKEYS inesperado");
        }
        if msg != proto::SSH_MSG_KEX_ECDH_INIT {
            return self.disconnect(DISCONNECT_PROTOCOL_ERROR, "se esperaba KEX_ECDH_INIT");
        }

        // Extraer Q_C.
        let mut r = Reader::new(payload);
        let _ = r.get_u8()?; // 30
        let q_c = r.get_string()?;

        // Ejecutar el kex del lado servidor.
        let v_s = proto::IDENT.as_bytes();
        let result = kex::run_server(
            &mut self.rng,
            host.signing(),
            &self.v_c,
            v_s,
            &self.i_c,
            &self.i_s,
            q_c,
        );
        let kx = match result {
            Ok(k) => k,
            Err(_) => return self.disconnect(DISCONNECT_KEY_EXCHANGE_FAILED, "fallo de kex"),
        };

        // session_id = H del PRIMER kex (permanece fijo aunque haya rekeys).
        if self.session_id.is_none() {
            self.session_id = Some(kx.h);
        }
        let session_id = self.session_id.unwrap();

        // Enviar KEX_ECDH_REPLY (en claro): K_S || Q_S || sig(H).
        let mut w = Writer::new();
        w.put_u8(proto::SSH_MSG_KEX_ECDH_REPLY)
            .put_string(&kx.k_s)
            .put_string(&kx.q_s)
            .put_string(&kx.sig_blob);
        let reply = w.into_bytes();
        self.send_packet(&reply)?;

        // Enviar NEWKEYS (en claro). A partir del SIGUIENTE envío ciframos.
        let mut nk = Writer::new();
        nk.put_u8(proto::SSH_MSG_NEWKEYS);
        let nk = nk.into_bytes();
        self.send_packet(&nk)?;

        // Derivar claves (RFC 4253 §7.2): sólo hacen falta las de cifrado.
        //  C = enc key c2s (nosotros abrimos), D = enc key s2c (nosotros sellamos).
        let mut c_key = [0u8; 64];
        let mut d_key = [0u8; 64];
        kex::derive_key(&kx.k_mpint, &kx.h, b'C', &session_id, &mut c_key);
        kex::derive_key(&kx.k_mpint, &kx.h, b'D', &session_id, &mut d_key);
        self.enc_in = Some(Aead::new(&c_key));
        self.enc_out = Some(Aead::new(&d_key));

        // Nuestro sentido de envío pasa a cifrado tras enviar NEWKEYS.
        self.out_encrypted = true;
        // Esperamos el NEWKEYS del cliente para cifrar el sentido de recepción.
        self.awaiting_client_newkeys = true;
        Ok(())
    }

    fn on_service_request(&mut self, payload: &[u8], msg: u8) -> KResult<()> {
        if msg != proto::SSH_MSG_SERVICE_REQUEST {
            return self.disconnect(DISCONNECT_PROTOCOL_ERROR, "se esperaba SERVICE_REQUEST");
        }
        let mut r = Reader::new(payload);
        let _ = r.get_u8()?;
        let service = r.get_string()?;
        if service != b"ssh-userauth" {
            return self.disconnect(DISCONNECT_BY_APPLICATION, "servicio no soportado");
        }
        let mut w = Writer::new();
        w.put_u8(proto::SSH_MSG_SERVICE_ACCEPT)
            .put_string(b"ssh-userauth");
        let p = w.into_bytes();
        self.send_packet(&p)?;
        self.state = State::UserAuth;
        Ok(())
    }

    fn on_userauth(&mut self, payload: &[u8], msg: u8) -> KResult<()> {
        if msg != proto::SSH_MSG_USERAUTH_REQUEST {
            // Ignorar otros mensajes en esta fase.
            return Ok(());
        }
        let mut r = Reader::new(payload);
        let _ = r.get_u8()?;
        let user = str::from_utf8(r.get_string()?)
            .map_err(|_| KError::InvalidArgument)?
            .to_string();
        let _service = r.get_string()?; // "ssh-connection"
        let method = r.get_string()?;

        let session_id = match &self.session_id {
            Some(s) => *s,
            None => return self.disconnect(DISCONNECT_PROTOCOL_ERROR, "sin session_id"),
        };

        match method {
            b"password" => {
                let _ = r.get_bool()?; // FALSE (sin cambio de contraseña)
                let password = r.get_string()?;
                match auth::check_password(&user, password) {
                    auth::AuthResult::Success => {
                        self.auth_user = user;
                        self.authed = true;
                        self.send_userauth_success()?;
                    }
                    _ => self.fail_auth()?,
                }
            }
            b"publickey" => {
                let has_sig = r.get_bool()?;
                let algo = r.get_string()?;
                let key_blob = r.get_string()?;
                if !has_sig {
                    // Sondeo: si la clave está autorizada, responder PK_OK.
                    if auth::probe_publickey(&user, algo, key_blob) {
                        let mut w = Writer::new();
                        w.put_u8(SSH_MSG_USERAUTH_PK_OK)
                            .put_string(algo)
                            .put_string(key_blob);
                        let p = w.into_bytes();
                        self.send_packet(&p)?;
                    } else {
                        self.fail_auth()?;
                    }
                } else {
                    let signature = r.get_string()?;
                    match auth::verify_publickey(&user, algo, key_blob, signature, &session_id) {
                        auth::AuthResult::Success => {
                            self.auth_user = user;
                            self.authed = true;
                            self.send_userauth_success()?;
                        }
                        _ => self.fail_auth()?,
                    }
                }
            }
            // "none" y cualquier otro método -> FAILURE con la lista de métodos.
            _ => self.fail_auth()?,
        }
        Ok(())
    }

    fn send_userauth_success(&mut self) -> KResult<()> {
        let mut w = Writer::new();
        w.put_u8(proto::SSH_MSG_USERAUTH_SUCCESS);
        let p = w.into_bytes();
        self.send_packet(&p)?;
        self.state = State::Session;
        Ok(())
    }

    fn fail_auth(&mut self) -> KResult<()> {
        self.auth_fails += 1;
        // FAILURE: name-list de métodos que se pueden seguir intentando + partial.
        let mut w = Writer::new();
        w.put_u8(proto::SSH_MSG_USERAUTH_FAILURE)
            .put_name_list(&["publickey", "password"])
            .put_bool(false);
        let p = w.into_bytes();
        self.send_packet(&p)?;
        if self.auth_fails >= MAX_AUTH_ATTEMPTS {
            return self.disconnect(DISCONNECT_BY_APPLICATION, "demasiados intentos");
        }
        Ok(())
    }

    fn on_session(&mut self, payload: &[u8], msg: u8) -> KResult<()> {
        match msg {
            proto::SSH_MSG_CHANNEL_OPEN => self.on_channel_open(payload),
            proto::SSH_MSG_CHANNEL_REQUEST => self.on_channel_request(payload),
            proto::SSH_MSG_CHANNEL_DATA => self.on_channel_data(payload),
            proto::SSH_MSG_CHANNEL_WINDOW_ADJUST => {
                let mut r = Reader::new(payload);
                let _ = r.get_u8()?;
                let _recipient = r.get_u32()?;
                let add = r.get_u32()?;
                if let Some(ch) = self.channel.as_mut() {
                    ch.add_send_window(add);
                }
                Ok(())
            }
            proto::SSH_MSG_CHANNEL_EOF => Ok(()), // el cliente no enviará más datos
            proto::SSH_MSG_CHANNEL_CLOSE => {
                // Responder CLOSE, cerrar el puente y la conexión.
                if let Some(ch) = self.channel.as_ref() {
                    let remote_id = ch.remote_id;
                    let mut w = Writer::new();
                    w.put_u8(proto::SSH_MSG_CHANNEL_CLOSE).put_u32(remote_id);
                    let p = w.into_bytes();
                    self.send_packet(&p)?;
                }
                remote::bridge_close();
                self.state = State::Closed;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn on_channel_open(&mut self, payload: &[u8]) -> KResult<()> {
        let mut r = Reader::new(payload);
        let _ = r.get_u8()?;
        let ch_type = r.get_string()?;
        let remote_id = r.get_u32()?;
        let peer_window = r.get_u32()?;
        let peer_max_packet = r.get_u32()?;

        if ch_type != b"session" || self.channel.is_some() {
            // Rechazar: sólo un canal `session` en el MVP.
            let mut w = Writer::new();
            w.put_u8(proto::SSH_MSG_CHANNEL_OPEN_FAILURE)
                .put_u32(remote_id)
                .put_u32(3) // SSH_OPEN_UNKNOWN_CHANNEL_TYPE / administratively prohibited
                .put_string(b"solo se soporta un canal session")
                .put_string(b"");
            let p = w.into_bytes();
            return self.send_packet(&p);
        }

        let local_id = 0u32;
        let ch = Channel::new(local_id, remote_id, peer_window, peer_max_packet);
        self.channel = Some(ch);

        let mut w = Writer::new();
        w.put_u8(proto::SSH_MSG_CHANNEL_OPEN_CONFIRMATION)
            .put_u32(remote_id) // recipient_channel
            .put_u32(local_id) // sender_channel
            .put_u32(channel::INITIAL_WINDOW)
            .put_u32(channel::MAX_CHANNEL_PACKET);
        let p = w.into_bytes();
        self.send_packet(&p)
    }

    fn on_channel_request(&mut self, payload: &[u8]) -> KResult<()> {
        let mut r = Reader::new(payload);
        let _ = r.get_u8()?;
        let _recipient = r.get_u32()?;
        let req_type = r.get_string()?.to_vec();
        let want_reply = r.get_bool()?;

        // Para pty-req extraemos TERM y dimensiones; para el resto, ignoramos.
        let (cols, rows) = if req_type == b"pty-req" {
            let _term = r.get_string()?;
            let cols = r.get_u32()?;
            let rows = r.get_u32()?;
            (cols, rows)
        } else {
            (0, 0)
        };

        let (remote_id, success) = match self.channel.as_mut() {
            Some(ch) => (ch.remote_id, ch.on_request(&req_type, cols, rows)),
            None => return Ok(()),
        };

        if want_reply {
            let mut w = Writer::new();
            let code = if success {
                proto::SSH_MSG_CHANNEL_SUCCESS
            } else {
                proto::SSH_MSG_CHANNEL_FAILURE
            };
            w.put_u8(code).put_u32(remote_id);
            let p = w.into_bytes();
            self.send_packet(&p)?;
        }
        Ok(())
    }

    fn on_channel_data(&mut self, payload: &[u8]) -> KResult<()> {
        let mut r = Reader::new(payload);
        let _ = r.get_u8()?;
        let _recipient = r.get_u32()?;
        let data = r.get_string()?;

        let adjust = match self.channel.as_mut() {
            Some(ch) => ch.on_data(data)?,
            None => return Ok(()),
        };
        if let Some(add) = adjust {
            let remote_id = self.channel.as_ref().unwrap().remote_id;
            let mut w = Writer::new();
            w.put_u8(proto::SSH_MSG_CHANNEL_WINDOW_ADJUST)
                .put_u32(remote_id)
                .put_u32(add);
            let p = w.into_bytes();
            self.send_packet(&p)?;
        }
        Ok(())
    }

    /// Empuja la salida pendiente de la shell como CHANNEL_DATA, respetando la
    /// ventana de envío y el tamaño máximo de paquete del cliente.
    fn pump_shell_output(&mut self) -> KResult<()> {
        let (remote_id, mut window, maxp, started) = match self.channel.as_ref() {
            Some(ch) => (ch.remote_id, ch.send_window, ch.peer_max_packet, ch.shell_started),
            None => return Ok(()),
        };
        if !started {
            return Ok(());
        }
        loop {
            if window == 0 || !remote::bridge_has_output() {
                break;
            }
            // Reservamos margen para la cabecera CHANNEL_DATA dentro de maxp.
            let cap = core::cmp::min(window as usize, maxp.saturating_sub(64) as usize).min(4096);
            if cap == 0 {
                break;
            }
            let data = remote::bridge_take_output(cap);
            if data.is_empty() {
                break;
            }
            let mut w = Writer::new();
            w.put_u8(proto::SSH_MSG_CHANNEL_DATA)
                .put_u32(remote_id)
                .put_string(&data);
            let p = w.into_bytes();
            self.send_packet(&p)?;
            window = window.saturating_sub(data.len() as u32);
        }
        if let Some(ch) = self.channel.as_mut() {
            ch.send_window = window;
        }
        Ok(())
    }

    /// Envía DISCONNECT y marca la conexión como cerrada.
    fn disconnect(&mut self, reason: u32, desc: &str) -> KResult<()> {
        let mut w = Writer::new();
        w.put_u8(proto::SSH_MSG_DISCONNECT)
            .put_u32(reason)
            .put_string(desc.as_bytes())
            .put_string(b""); // language tag
        let p = w.into_bytes();
        // Ignoramos el error de envío: vamos a cerrar de todos modos.
        let _ = self.send_packet(&p);
        remote::bridge_close();
        self.state = State::Closed;
        Ok(())
    }
}

/// Imprime por consola la huella de la clave de host (para `known_hosts`).
pub fn announce_host_key(host: &HostKey) {
    println!(
        "[ssh] host key ssh-ed25519 fingerprint: {}",
        host.fingerprint()
    );
}
