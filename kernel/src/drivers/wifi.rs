#![allow(dead_code, unused_imports)]

use core::sync::atomic::{AtomicU8, Ordering};

use esp_println::println;

use crate::prelude::*;
use crate::scheduler;

use crate::arch::xtensa::sync::Mutex;

use crate::arch::xtensa::timer::uptime_ms;

use crate::wifi_credentials::{WIFI_PASSWORD, WIFI_SSID};

use esp_hal::peripherals::{RADIO_CLK, RNG, TIMG0, WIFI};
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;

use esp_wifi::wifi::{
    self, AuthMethod, ClientConfiguration, Configuration, WifiController, WifiDevice, WifiStaDevice,
};
use esp_wifi::EspWifiController;

use smoltcp::iface::{Config as IfaceConfig, Interface, SocketSet};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};

use crate::drivers::ssh::crypto_rng::HwRng;
use crate::drivers::ssh::{Connection, HostKey, Transport, SSH_PORT};

pub const ECHO_PORT: u16 = 2323;

const ECHO_RX_SIZE: usize = 2048;
const ECHO_TX_SIZE: usize = 2048;

const ECHO_CHUNK: usize = 512;

const LINK_CHECK_MS: u64 = 5_000;

const SSH_RX_SIZE: usize = 4096;
const SSH_TX_SIZE: usize = 4096;

/// Puerto de recepción de imágenes OTA (Fase 5). La imagen se bufferea en PSRAM;
/// el flasheo real se dispara con `ota apply` desde la shell.
pub const OTA_PORT: u16 = 3300;
const OTA_RX_SIZE: usize = 8192;
const OTA_TX_SIZE: usize = 1024;

pub enum NetCmd {
    Connect {
        handle: smoltcp::iface::SocketHandle,
        ip: [u8; 4],
        port: u16,
    },
}

pub static NET_SOCKETS: Mutex<Option<SocketSet<'static>>> = Mutex::new(None);
pub static NET_CMD_QUEUE: Mutex<Vec<NetCmd>> = Mutex::new(Vec::new());

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

    let init: &'static EspWifiController<'static> = Box::leak(Box::new(init));

    let (mut device, mut controller): (WifiDevice<'static, WifiStaDevice>, WifiController<'static>) =
        match wifi::new_with_mode(init, periph.wifi, WifiStaDevice) {
            Ok(v) => v,
            Err(e) => {
                println!("[net] ERROR new_with_mode: {:?}", e);
                set_status(WifiStatus::Failed);
                return;
            }
        };

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

    if let Err(e) = controller.start() {
        println!("[net] ERROR controller.start: {:?}", e);
        set_status(WifiStatus::Failed);
        return;
    }
    set_status(WifiStatus::Connecting);
    println!("[net] conectando a SSID '{}'...", WIFI_SSID);
    let _ = controller.connect();

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

    let mac = device.mac_address();
    let mut if_cfg = IfaceConfig::new(HardwareAddress::Ethernet(EthernetAddress::from_bytes(&mac)));

    if_cfg.random_seed = uptime_ms().wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut iface = Interface::new(if_cfg, &mut device, now_smoltcp());

    let mut sockets_set = SocketSet::new(Vec::new());
    let dhcp_handle = sockets_set.add(dhcpv4::Socket::new());

    let echo_rx = tcp::SocketBuffer::new(alloc::vec![0u8; ECHO_RX_SIZE]);
    let echo_tx = tcp::SocketBuffer::new(alloc::vec![0u8; ECHO_TX_SIZE]);
    let mut echo = tcp::Socket::new(echo_rx, echo_tx);
    if let Err(e) = echo.listen(ECHO_PORT) {
        println!("[net] ERROR listen({}): {:?}", ECHO_PORT, e);
        set_status(WifiStatus::Failed);
        return;
    }
    let echo_handle = sockets_set.add(echo);

    let ssh_rx = tcp::SocketBuffer::new(alloc::vec![0u8; SSH_RX_SIZE]);
    let ssh_tx = tcp::SocketBuffer::new(alloc::vec![0u8; SSH_TX_SIZE]);
    let mut ssh_sock = tcp::Socket::new(ssh_rx, ssh_tx);
    if let Err(e) = ssh_sock.listen(SSH_PORT) {
        println!("[ssh] ERROR listen({}): {:?}", SSH_PORT, e);
    }
    let ssh_handle = sockets_set.add(ssh_sock);

    // Socket de recepción OTA (Fase 5): bufferea la imagen en PSRAM.
    let ota_rx = tcp::SocketBuffer::new(alloc::vec![0u8; OTA_RX_SIZE]);
    let ota_tx = tcp::SocketBuffer::new(alloc::vec![0u8; OTA_TX_SIZE]);
    let mut ota_sock = tcp::Socket::new(ota_rx, ota_tx);
    if let Err(e) = ota_sock.listen(OTA_PORT) {
        println!("[ota] ERROR listen({}): {:?}", OTA_PORT, e);
    }
    let ota_handle = sockets_set.add(ota_sock);
    let mut ota_receiving = false;

    let host_key = HostKey::from_seed(&crate::drivers::ssh::config::HOST_KEY_SEED);

    let mut ssh_conn = Connection::new(HwRng::new(rng));
    let mut ssh_active = false;

    let mut have_ip = false;
    let mut next_link_check = uptime_ms().saturating_add(LINK_CHECK_MS);
    let mut buf = [0u8; ECHO_CHUNK];

    *NET_SOCKETS.lock() = Some(sockets_set);

    loop {
        {
            let t = now_smoltcp();
            
            // Procesar comandos encolados
            let mut cmds = Vec::new();
            {
                let mut q = NET_CMD_QUEUE.lock();
                cmds.extend(q.drain(..));
            }
            
            let mut sockets_guard = NET_SOCKETS.lock();
            let sockets = sockets_guard.as_mut().unwrap();
            
            for cmd in cmds {
                match cmd {
                    NetCmd::Connect { handle, ip, port } => {
                        let sock = sockets.get_mut::<tcp::Socket>(handle);
                        let remote_addr = smoltcp::wire::IpAddress::Ipv4(smoltcp::wire::Ipv4Address::from_octets(ip));
                        let remote_endpoint = smoltcp::wire::IpEndpoint::new(remote_addr, port);
                        let local_port = 49152 + (uptime_ms() % 16384) as u16;
                        let _ = sock.connect(iface.context(), remote_endpoint, local_port);
                    }
                }
            }
            
            let _ = iface.poll(t, &mut device, sockets);

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
                            println!("[net] IP = {}", ip);
                            println!(
                                "[net] SSH escuchando en puerto {}, ECHO en {}, OTA en {}",
                                SSH_PORT, ECHO_PORT, OTA_PORT
                            );
                            set_status(WifiStatus::Connected);
                        }
                        have_ip = true;
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

            {
                let sock = sockets.get_mut::<tcp::Socket>(echo_handle);

                if !sock.is_open() {
                    let _ = sock.listen(ECHO_PORT);
                }

                if sock.can_recv() {
                    if let Ok(n) = sock.recv_slice(&mut buf) {
                        if n > 0 && sock.can_send() {
                            let _ = sock.send_slice(&buf[..n]);
                        }
                    }
                }

                if sock.may_send() && !sock.may_recv() {
                    sock.close();
                }
            }

            {
                let sock = sockets.get_mut::<tcp::Socket>(ssh_handle);

                if !sock.is_open() {
                    let _ = sock.listen(SSH_PORT);
                    ssh_active = false;
                }
                if sock.is_active() && !ssh_active {
                    ssh_conn = Connection::new(HwRng::new(rng));
                    ssh_active = true;
                }
                if ssh_active {
                    let mut transport = TcpTransport { sock };
                    match ssh_conn.pump(&mut transport, &host_key) {
                        Ok(()) => {
                            if ssh_conn.is_closed() {
                                transport.close();
                                ssh_active = false;
                            }
                        }
                        Err(e) => {
                            if e != KError::WouldBlock {
                                println!("[ssh] pump ERROR en estado {:?}: {:?}", ssh_conn.state(), e);
                                transport.close();
                                ssh_active = false;
                            }
                        }
                    }
                }
            }

            {
                let sock = sockets.get_mut::<tcp::Socket>(ota_handle);

                if !sock.is_open() {
                    let _ = sock.listen(OTA_PORT);
                    ota_receiving = false;
                }
                if sock.is_active() {
                    if !ota_receiving {
                        crate::ota::rx_begin();
                        ota_receiving = true;
                        println!("[ota] recibiendo imagen en el puerto {}...", OTA_PORT);
                    }
                    if sock.can_recv() {
                        if let Ok(n) = sock.recv_slice(&mut buf) {
                            if n > 0 {
                                if let Err(e) = crate::ota::rx_push(&buf[..n]) {
                                    println!("[ota] ERROR al bufferear: {:?}", e);
                                    crate::ota::rx_clear();
                                    sock.close();
                                    ota_receiving = false;
                                }
                            }
                        }
                    }
                    if ota_receiving && !sock.may_recv() {
                        let total = crate::ota::rx_len();
                        println!(
                            "[ota] imagen recibida: {} bytes. Flashea con 'ota apply' (mejor por consola).",
                            total
                        );
                        sock.close();
                        ota_receiving = false;
                    }
                }
            }

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
        } // guard drops here

        scheduler::yield_now();
    }
}

