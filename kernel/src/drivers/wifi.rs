//! Controlador WiFi: binding de `esp-wifi` (radio 802.11) + `smoltcp` (TCP/IP).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Une tres piezas para dar red al kernel:
//!   1. La radio 802.11 y su firmware, gestionados por `esp-wifi` 0.12.
//!   2. El stack TCP/IP `no_std` `smoltcp` 0.12, alimentado por el `WifiDevice`
//!      que expone `esp-wifi` (implementa `smoltcp::phy::Device`).
//!   3. Un cliente DHCPv4 para obtener IP automáticamente.
//!
//! META DE ESTA FASE (verificable con `nc <ip> 2323` desde el host):
//!   (1) STA conecta a `WIFI_SSID`/`WIFI_PASSWORD` (2.4 GHz);
//!   (2) DHCP asigna IP e imprimimos `[net] IP = a.b.c.d`;
//!   (3) levantamos un servidor TCP de ECO en el puerto 2323.
//!
//! DECISIÓN DE INTEGRACIÓN (propiedad de periféricos):
//!   `esp-wifi` necesita poseer WIFI, RADIO_CLK, un timer (TIMG0) y RNG, que solo
//!   `main` tiene tras `esp_hal::init`. Como el `spawn` del scheduler solo admite
//!   `fn(usize)`, `main` deposita esos cuatro periféricos en el `static PENDING`
//!   ANTES de arrancar el scheduler (`provide_peripherals`). La tarea de red
//!   [`net_task`] los recoge y hace TODA la bring-up (init firmware + asociación +
//!   smoltcp + DHCP + eco) dentro de su propio bucle, cediendo con
//!   `scheduler::yield_now()`. Así todo el estado de red vive en la pila de la
//!   tarea (no hace falta `Mutex` global) y no se rompen shell/heartbeat.
//!
//! AVISO: alto riesgo — sin compilar contra hardware. Los puntos de API que no se
//! pueden confirmar al 100% van marcados con `// (?)`.
#![allow(dead_code, unused_imports)]

use core::sync::atomic::{AtomicU8, Ordering};

use esp_println::println;

use crate::prelude::*;
use crate::scheduler;
// Mutex canónico del kernel (§3.2.4). Enmascara IRQs mientras se mantiene tomado,
// por eso SOLO se usa para el traspaso breve de periféricos (nunca a través de un
// `yield_now`).
use crate::arch::xtensa::sync::Mutex;
// Reloj monotónico del kernel (ms) para backoffs/temporizadores gruesos.
use crate::arch::xtensa::timer::uptime_ms;
// Credenciales de la red de desarrollo (main declara `mod wifi_credentials;`).
use crate::wifi_credentials::{WIFI_PASSWORD, WIFI_SSID};

// -- Periféricos de esp-hal que consume el init de esp-wifi. --
use esp_hal::peripherals::{RADIO_CLK, RNG, TIMG0, WIFI};
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;

// -- API de esp-wifi 0.12.0 (coherente con esp-hal 0.23.x). --
use esp_wifi::wifi::{
    self, AuthMethod, ClientConfiguration, Configuration, WifiController, WifiDevice, WifiStaDevice,
};
use esp_wifi::EspWifiController;

// -- API de smoltcp 0.12.0. --
use smoltcp::iface::{Config as IfaceConfig, Interface, SocketSet};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};

// -- Servidor SSH sobre el mismo stack smoltcp (listener en el puerto 22). --
use crate::drivers::ssh::crypto_rng::HwRng;
use crate::drivers::ssh::{Connection, HostKey, Transport, SSH_PORT};

// ============================================================================
// Parámetros.
// ============================================================================

/// Puerto del servidor de ECO (META de esta fase).
pub const ECHO_PORT: u16 = 2323;
/// Búferes RX/TX del socket de eco (bytes).
const ECHO_RX_SIZE: usize = 2048;
const ECHO_TX_SIZE: usize = 2048;
/// Búfer de trabajo para el eco (copia RX->TX por pasada).
const ECHO_CHUNK: usize = 512;
/// Periodo del chequeo de enlace/reconexión (ms).
const LINK_CHECK_MS: u64 = 5_000;

/// Búferes RX/TX del socket SSH (puerto 22). En el MVP los paquetes SSH son
/// pequeños (kex + datos de canal), así que 4 KB por sentido bastan.
const SSH_RX_SIZE: usize = 4096;
const SSH_TX_SIZE: usize = 4096;

/// Adaptador `Transport` (lo consume la máquina de estados SSH) sobre un socket
/// TCP de smoltcp. `Ok(0)` = sin datos / sin sitio ahora (NO bloqueante).
struct TcpTransport<'s> {
    sock: &'s mut tcp::Socket<'static>,
}
impl<'s> Transport for TcpTransport<'s> {
    fn read(&mut self, buf: &mut [u8]) -> KResult<usize> {
        if !self.sock.can_recv() {
            return Ok(0);
        }
        self.sock.recv_slice(buf).map_err(|_| KError::IoError)
    }
    fn write(&mut self, buf: &[u8]) -> KResult<usize> {
        if !self.sock.can_send() {
            return Ok(0);
        }
        self.sock.send_slice(buf).map_err(|_| KError::IoError)
    }
    fn close(&mut self) {
        self.sock.close();
    }
}

// ============================================================================
// Estado del enlace — visible por `status()` (lectura sin bloqueo).
// ============================================================================

/// Estado del enlace WiFi (contrato §3.9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WifiStatus {
    /// Radio apagada o sin configurar.
    Down,
    /// Asociándose al AP / negociando DHCP.
    Connecting,
    /// Asociado y con dirección IP asignada.
    Connected,
    /// El último intento de conexión falló.
    Failed,
}

const ST_DOWN: u8 = 0;
const ST_CONNECTING: u8 = 1;
const ST_CONNECTED: u8 = 2;
const ST_FAILED: u8 = 3;

static STATUS: AtomicU8 = AtomicU8::new(ST_DOWN);

fn set_status(s: WifiStatus) {
    let v = match s {
        WifiStatus::Down => ST_DOWN,
        WifiStatus::Connecting => ST_CONNECTING,
        WifiStatus::Connected => ST_CONNECTED,
        WifiStatus::Failed => ST_FAILED,
    };
    STATUS.store(v, Ordering::Release);
}

/// Estado actual del enlace (contrato §3.9). Lectura sin bloqueo, usable desde la
/// shell u otros subsistemas.
pub fn status() -> WifiStatus {
    match STATUS.load(Ordering::Acquire) {
        ST_CONNECTING => WifiStatus::Connecting,
        ST_CONNECTED => WifiStatus::Connected,
        ST_FAILED => WifiStatus::Failed,
        _ => WifiStatus::Down,
    }
}

// ============================================================================
// Traspaso de periféricos main -> net_task.
// ============================================================================

/// Contenedor de los periféricos que `esp-wifi` necesita poseer.
struct NetPeripherals {
    timg0: TIMG0,
    rng: RNG,
    radio_clk: RADIO_CLK,
    wifi: WIFI,
}

// SEGURIDAD: los singletons de periférico se mueven UNA sola vez (main -> static
// -> net_task) en un sistema monociclo; el `Mutex` serializa el acceso. Marcamos
// el contenedor `Send` para poder alojarlo en el `static Mutex`.
unsafe impl Send for NetPeripherals {}

/// Buzón de periféricos: `main` lo llena, `net_task` lo vacía.
static PENDING: Mutex<Option<NetPeripherals>> = Mutex::new(None);

/// `main` deposita aquí los periféricos de red ANTES de `scheduler::run()`.
///
/// Ejemplo (en `main`, tras `esp_hal::init`):
/// ```ignore
/// drivers::wifi::provide_peripherals(
///     peripherals.TIMG0, peripherals.RNG, peripherals.RADIO_CLK, peripherals.WIFI,
/// );
/// ```
pub fn provide_peripherals(timg0: TIMG0, rng: RNG, radio_clk: RADIO_CLK, wifi: WIFI) {
    let mut g = PENDING.lock();
    *g = Some(NetPeripherals {
        timg0,
        rng,
        radio_clk,
        wifi,
    });
}

// ============================================================================
// Fuente de tiempo para smoltcp (µs, resolución 1 µs vía SYSTIMER).
// ============================================================================

/// Marca de tiempo para smoltcp. Usa el reloj del HAL en microsegundos (Opción A
/// de la referencia): resolución fina para los temporizadores/RTO de TCP.
#[inline]
fn now_smoltcp() -> Instant {
    // (?) cadena `now().duration_since_epoch().to_micros()`: idéntica a la ya
    // usada por `arch::xtensa::timer::uptime_ms` (con `to_millis`), así que la ruta
    // está confirmada en este árbol.
    let us = esp_hal::time::now().duration_since_epoch().to_micros();
    Instant::from_micros(us as i64)
}

// ============================================================================
// Tarea de red — punto de entrada del scheduler (`fn(usize)`).
// ============================================================================

/// Cuerpo de la tarea de red. Hace TODA la bring-up y luego bombea el stack en un
/// bucle que cede la CPU. NUNCA panica: ante cualquier fallo de bring-up imprime
/// el motivo, deja `status()==Failed` y sale (el scheduler la reaparea como
/// zombie sin afectar a shell/heartbeat).
pub fn net_task(_arg: usize) {
    set_status(WifiStatus::Down);

    // -- 1. Recuperar los periféricos que dejó `main`. --
    let periph = { PENDING.lock().take() };
    let periph = match periph {
        Some(p) => p,
        None => {
            println!("[net] ERROR: periféricos no provistos (¿faltó provide_peripherals?)");
            set_status(WifiStatus::Failed);
            return;
        }
    };

    // -- 2. Inicializar el firmware de radio. --
    // esp-wifi usa un timer (TIMG0.timer0) para su planificador interno y un RNG
    // para la entropía WPA. Requiere el heap ya inicializado (lo está: `main` hizo
    // `mm::heap::init` + PSRAM) y CPU >= 80 MHz (main fija CpuClock::max()).
    let timg0 = TimerGroup::new(periph.timg0);
    let rng = Rng::new(periph.rng);

    let init = match esp_wifi::init(timg0.timer0, rng, periph.radio_clk) {
        Ok(c) => c,
        Err(e) => {
            println!("[net] ERROR esp_wifi::init: {:?}", e);
            set_status(WifiStatus::Failed);
            return;
        }
    };
    // Fugamos el `EspWifiController` a `'static` (patrón `mk_static!`): debe
    // sobrevivir al par WiFi porque `new_with_mode` lo toma por referencia `&'d`.
    // Vive lo que dura el SO; no se libera nunca.
    let init: &'static EspWifiController<'static> = Box::leak(Box::new(init));

    // -- 3. Crear el par (dispositivo de enlace, controlador) en modo estación. --
    let (mut device, mut controller): (WifiDevice<'static, WifiStaDevice>, WifiController<'static>) =
        match wifi::new_with_mode(init, periph.wifi, WifiStaDevice) {
            Ok(v) => v,
            Err(e) => {
                println!("[net] ERROR new_with_mode: {:?}", e);
                set_status(WifiStatus::Failed);
                return;
            }
        };

    // -- 4. Configurar la STA con las credenciales. --
    // Los strings de esp-wifi son `heapless::String<32>`/`<64>`; convertimos con
    // `try_into` (SSID máx 32, password máx 64).
    let ssid_h = match WIFI_SSID.try_into() {
        Ok(s) => s,
        Err(_) => {
            println!("[net] ERROR: SSID demasiado largo (>32)");
            set_status(WifiStatus::Failed);
            return;
        }
    };
    let pass_h = match WIFI_PASSWORD.try_into() {
        Ok(p) => p,
        Err(_) => {
            println!("[net] ERROR: password demasiado largo (>64)");
            set_status(WifiStatus::Failed);
            return;
        }
    };
    let client = ClientConfiguration {
        ssid: ssid_h,
        password: pass_h,
        auth_method: if WIFI_PASSWORD.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        ..Default::default()
    };
    if let Err(e) = controller.set_configuration(&Configuration::Client(client)) {
        println!("[net] ERROR set_configuration: {:?}", e);
        set_status(WifiStatus::Failed);
        return;
    }

    // -- 5. Arrancar la radio y lanzar la asociación. --
    if let Err(e) = controller.start() {
        println!("[net] ERROR controller.start: {:?}", e);
        set_status(WifiStatus::Failed);
        return;
    }
    set_status(WifiStatus::Connecting);
    println!("[net] conectando a SSID '{}'...", WIFI_SSID);
    let _ = controller.connect(); // (?) blocking en 0.12: inicia la asociación

    // -- 6. Esperar asociación a nivel de enlace, cediendo la CPU. --
    // `is_connected()`: Ok(true)=asociado, Ok(false)=en curso, Err(_)=desasociado
    // (reintentar). Reintentos throttled para no spamear al firmware.
    let mut next_retry = uptime_ms().saturating_add(2_000);
    loop {
        match controller.is_connected() {
            Ok(true) => break,
            Ok(false) => {}
            Err(_) => {
                if uptime_ms() >= next_retry {
                    next_retry = uptime_ms().saturating_add(2_000);
                    let _ = controller.connect();
                }
            }
        }
        scheduler::yield_now();
    }
    println!("[net] asociado al AP; negociando DHCP...");

    // -- 7. Construir la interfaz IP de smoltcp sobre el dispositivo. --
    let mac = device.mac_address();
    let mut if_cfg = IfaceConfig::new(HardwareAddress::Ethernet(EthernetAddress::from_bytes(&mac)));
    // Semilla aleatoria distinta en cada arranque (puertos/secuencias TCP).
    if_cfg.random_seed = uptime_ms().wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut iface = Interface::new(if_cfg, &mut device, now_smoltcp());

    // -- 8. Sockets: DHCPv4 + servidor TCP de eco escuchando en ECHO_PORT. --
    let mut sockets = SocketSet::new(Vec::new());
    let dhcp_handle = sockets.add(dhcpv4::Socket::new());

    let echo_rx = tcp::SocketBuffer::new(alloc::vec![0u8; ECHO_RX_SIZE]);
    let echo_tx = tcp::SocketBuffer::new(alloc::vec![0u8; ECHO_TX_SIZE]);
    let mut echo = tcp::Socket::new(echo_rx, echo_tx);
    if let Err(e) = echo.listen(ECHO_PORT) {
        println!("[net] ERROR listen({}): {:?}", ECHO_PORT, e);
        set_status(WifiStatus::Failed);
        return;
    }
    let echo_handle = sockets.add(echo);

    // -- Servidor SSH: socket en escucha en el puerto 22. --
    let ssh_rx = tcp::SocketBuffer::new(alloc::vec![0u8; SSH_RX_SIZE]);
    let ssh_tx = tcp::SocketBuffer::new(alloc::vec![0u8; SSH_TX_SIZE]);
    let mut ssh_sock = tcp::Socket::new(ssh_rx, ssh_tx);
    if let Err(e) = ssh_sock.listen(SSH_PORT) {
        println!("[ssh] ERROR listen({}): {:?}", SSH_PORT, e);
    }
    let ssh_handle = sockets.add(ssh_sock);

    // Host key ed25519 (generada al arranque con el TRNG; la radio está activa
    // aquí, así que HwRng es un CSPRNG válido). Se imprime su huella para known_hosts.
    let mut ssh_seed_rng = HwRng::new(rng);
    let host_key = HostKey::generate(&mut ssh_seed_rng);
    // Máquina de estados de la conexión SSH (una a la vez en el MVP).
    let mut ssh_conn = Connection::new(HwRng::new(rng));
    let mut ssh_active = false;

    // -- 9. Bucle principal de red: poll + DHCP + eco, cediendo la CPU. --
    let mut have_ip = false;
    let mut next_link_check = uptime_ms().saturating_add(LINK_CHECK_MS);
    let mut buf = [0u8; ECHO_CHUNK];

    loop {
        // (a) Una pasada del motor smoltcp: consume RX/TX del WifiDevice.
        let t = now_smoltcp();
        let _ = iface.poll(t, &mut device, &mut sockets); // devuelve PollResult (ignorado)

        // (b) Procesar el cliente DHCP y aplicar la configuración IP.
        // `dhcp_handle` es válido por construcción -> `get_mut` no panica. Extraemos
        // el evento (valor propio) para no solapar el préstamo de `sockets` con el de
        // `iface` (variables distintas, préstamos disjuntos).
        let event = sockets.get_mut::<dhcpv4::Socket>(dhcp_handle).poll();
        match event {
            Some(dhcpv4::Event::Configured(config)) => {
                iface.update_ip_addrs(|addrs| {
                    addrs.clear();
                    let _ = addrs.push(IpCidr::Ipv4(config.address));
                });
                if let Some(router) = config.router {
                    let _ = iface.routes_mut().add_default_ipv4_route(router);
                } else {
                    iface.routes_mut().remove_default_ipv4_route();
                }
                if !have_ip {
                    if let Some(ip) = iface.ipv4_addr() {
                        // META verificable: la línea que busca el operador en consola.
                        println!("[net] IP = {}", ip);
                        println!(
                            "[net] servidor de ECO TCP escuchando en el puerto {} (probar: nc {} {})",
                            ECHO_PORT, ip, ECHO_PORT
                        );
                        println!(
                            "[ssh] servidor SSH en el puerto {} (probar: ssh root@{})",
                            SSH_PORT, ip
                        );
                        println!("[ssh] host key: {}", host_key.fingerprint());
                    }
                    have_ip = true;
                    set_status(WifiStatus::Connected);
                }
            }
            Some(dhcpv4::Event::Deconfigured) => {
                iface.update_ip_addrs(|addrs| addrs.clear());
                iface.routes_mut().remove_default_ipv4_route();
                have_ip = false;
                if status() == WifiStatus::Connected {
                    set_status(WifiStatus::Connecting);
                }
            }
            None => {}
        }

        // (c) Servidor de ECO: patrón robusto (re-escucha + eco + medio cierre).
        {
            let sock = sockets.get_mut::<tcp::Socket>(echo_handle);

            // Tras un cierre completo del cliente, volver a escuchar.
            if !sock.is_open() {
                let _ = sock.listen(ECHO_PORT);
            }

            // Eco directo: devolver lo recibido (lo que quepa en TX).
            if sock.can_recv() {
                if let Ok(n) = sock.recv_slice(&mut buf) {
                    if n > 0 && sock.can_send() {
                        let _ = sock.send_slice(&buf[..n]);
                    }
                }
            }

            // El peer cerró su mitad (FIN): cerramos la nuestra para reciclar.
            if sock.may_send() && !sock.may_recv() {
                sock.close();
            }
        }

        // (c-bis) Servidor SSH: conduce la máquina de estados sobre el socket:22.
        {
            let sock = sockets.get_mut::<tcp::Socket>(ssh_handle);
            // Tras cerrarse una sesión, volver a escuchar y marcar inactiva.
            if !sock.is_open() {
                let _ = sock.listen(SSH_PORT);
                ssh_active = false;
            }
            if sock.is_active() {
                // Cliente nuevo -> arranca una máquina de estados SSH fresca.
                if !ssh_active {
                    ssh_conn = Connection::new(HwRng::new(rng));
                    ssh_active = true;
                }
                let mut transport = TcpTransport { sock };
                // `pump` hace TODO el trabajo disponible sin bloquear y regresa.
                if ssh_conn.pump(&mut transport, &host_key).is_err() {
                    transport.close(); // error de protocolo -> cerrar y reiniciar
                }
            }
        }

        // (d) Chequeo periódico de enlace: reconectar si se cayó la asociación.
        if uptime_ms() >= next_link_check {
            next_link_check = uptime_ms().saturating_add(LINK_CHECK_MS);
            match controller.is_connected() {
                Ok(true) => {}
                _ => {
                    set_status(WifiStatus::Connecting);
                    let _ = controller.connect();
                }
            }
        }

        // (e) Ceder la CPU: NO monopolizar el núcleo (shell/heartbeat siguen vivos).
        scheduler::yield_now();
    }
}

// ----------------------------------------------------------------------------
// PUNTOS A VERIFICAR CONTRA LOS CRATES RESUELTOS (marcados `(?)`):
//   * `esp_wifi::init(timer, rng, radio_clk) -> Result<EspWifiController, _>`.
//   * `wifi::new_with_mode(&EspWifiController, WIFI, WifiStaDevice)
//        -> Result<(WifiDevice, WifiController), _>`.
//   * `WifiController::{set_configuration,start,connect,is_connected}`.
//   * `WifiDevice::mac_address() -> [u8; 6]`.
//   * smoltcp: `Interface::poll` devuelve `PollResult` (no `bool`); `ipv4_addr`,
//     `update_ip_addrs`, `routes_mut`, `EthernetAddress::from_bytes`.
//   * `dhcpv4::Socket::poll() -> Option<Event>`; `Event::Configured(Config{address,router,..})`.
//   * tcp `Socket::{listen,is_open,can_recv,can_send,recv_slice,send_slice,may_send,may_recv,close}`.
// ----------------------------------------------------------------------------
