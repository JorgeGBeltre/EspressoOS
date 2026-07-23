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

pub enum WifiCmd {
    Scan,
    Connect { ssid: String, password: String },
    Disconnect,
}
pub static WIFI_CMD_QUEUE: Mutex<Vec<WifiCmd>> = Mutex::new(Vec::new());

#[derive(Clone)]
pub struct ApInfo {
    pub ssid: String,
    pub rssi: i8,
    pub channel: u8,
    pub secured: bool,
}
pub static SCAN_RESULTS: Mutex<Vec<ApInfo>> = Mutex::new(Vec::new());

pub const SCAN_IDLE: u8 = 0;
pub const SCAN_RUNNING: u8 = 1;
pub const SCAN_DONE: u8 = 2;
pub const SCAN_ERROR: u8 = 3;
static SCAN_STATE: AtomicU8 = AtomicU8::new(SCAN_IDLE);

/// Diagnóstico legible del último scan, visible desde la shell (sin necesidad del
/// log serial): p.ej. "started", "12 APs in 1800 ms", "error ...".
pub static SCAN_DIAG: Mutex<String> = Mutex::new(String::new());
pub fn scan_diag() -> String {
    SCAN_DIAG.lock().clone()
}

pub static CURRENT_IP: Mutex<Option<[u8; 4]>> = Mutex::new(None);
pub static CURRENT_SSID: Mutex<Option<String>> = Mutex::new(None);

pub fn request_scan() {
    *SCAN_DIAG.lock() = String::from("queued");
    SCAN_STATE.store(SCAN_RUNNING, Ordering::Release);
    WIFI_CMD_QUEUE.lock().push(WifiCmd::Scan);
}
pub fn scan_state() -> u8 {
    SCAN_STATE.load(Ordering::Acquire)
}
pub fn scan_results() -> Vec<ApInfo> {
    SCAN_RESULTS.lock().clone()
}

pub fn request_connect(ssid: String, password: String) {
    WIFI_CMD_QUEUE
        .lock()
        .push(WifiCmd::Connect { ssid, password });
}
pub fn request_disconnect() {
    WIFI_CMD_QUEUE.lock().push(WifiCmd::Disconnect);
}
pub fn current_ip() -> Option<[u8; 4]> {
    *CURRENT_IP.lock()
}
pub fn current_ssid() -> Option<String> {
    CURRENT_SSID.lock().clone()
}

// ---- /dev/wlan0: control de wifi por ioctl + estado por read() (SP2, R2) ----

/// ioctl cmds de `/dev/wlan0` (espejados en `/bin/wifi`). D-3: ioctl = órdenes, read =
/// estado. `connect` auto-persiste en NVS (lo hace `process_wifi_cmd`), como el builtin.
/// No-op: valida el camino "ioctl aceptado" sin efectos (para `/bin/ioctltest`), sin
/// tener que disparar un connect/scan real que tiraría la red.
pub const WLAN_NOP: u32 = 0;
pub const WLAN_CONNECT: u32 = 1;
pub const WLAN_DISCONNECT: u32 = 2;
pub const WLAN_SCAN: u32 = 3;

/// Límites de campo (D-2), impuestos aquí en el kernel: SSID ≤ 32 (802.11), pass WPA ≤ 64.
const WLAN_SSID_MAX: usize = 32;
const WLAN_PASS_MAX: usize = 64;

/// Struct que userland pasa por `arg` del ioctl CONNECT (D-1, struct tipado). Campos en
/// `usize` (32-bit en Xtensa): `{ssid_ptr, ssid_len, pass_ptr, pass_len}`. El kernel valida
/// el struct Y cada puntero interno con `validate_user`.
#[repr(C)]
struct WlanConnectReq {
    ssid_ptr: usize,
    ssid_len: usize,
    pass_ptr: usize,
    pass_len: usize,
}

struct WlanDevice;

impl crate::vfs::devfs::Device for WlanDevice {
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let snap = wlan_snapshot();
        let bytes = snap.as_bytes();
        let start = off as usize;
        if start >= bytes.len() {
            return Ok(0);
        }
        let n = core::cmp::min(bytes.len() - start, buf.len());
        buf[..n].copy_from_slice(&bytes[start..start + n]);
        Ok(n)
    }

    fn write(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }

    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            WLAN_NOP => Ok(0),
            WLAN_CONNECT => {
                // D-1: valida el struct Y cada puntero interno antes de leer memoria de
                // usuario. `validate_user` es la misma barrera que usan read/write/spawn.
                crate::syscall::handler::validate_user(
                    arg,
                    core::mem::size_of::<WlanConnectReq>(),
                )?;
                let req = unsafe { &*(arg as *const WlanConnectReq) };
                if req.ssid_len == 0
                    || req.ssid_len > WLAN_SSID_MAX
                    || req.pass_len > WLAN_PASS_MAX
                {
                    return Err(KError::InvalidArgument);
                }
                crate::syscall::handler::validate_user(req.ssid_ptr, req.ssid_len)?;
                crate::syscall::handler::validate_user(req.pass_ptr, req.pass_len)?;
                let ssid_bytes =
                    unsafe { core::slice::from_raw_parts(req.ssid_ptr as *const u8, req.ssid_len) };
                let pass_bytes =
                    unsafe { core::slice::from_raw_parts(req.pass_ptr as *const u8, req.pass_len) };
                let ssid = core::str::from_utf8(ssid_bytes).map_err(|_| KError::InvalidArgument)?;
                let pass = core::str::from_utf8(pass_bytes).map_err(|_| KError::InvalidArgument)?;
                request_connect(String::from(ssid), String::from(pass));
                Ok(0)
            }
            WLAN_DISCONNECT => {
                request_disconnect();
                Ok(0)
            }
            WLAN_SCAN => {
                request_scan();
                Ok(0)
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

/// Snapshot legible de wlan0 + último scan (D-3). Formato estable que leen `/bin/wifi`,
/// `/bin/ip` y `/bin/nmcli`.
fn wlan_snapshot() -> String {
    let st = match status() {
        WifiStatus::Down => "Down",
        WifiStatus::Connecting => "Connecting",
        WifiStatus::Connected => "Connected",
        WifiStatus::Failed => "Failed",
    };
    let ssid = current_ssid().unwrap_or_else(|| String::from("-"));
    let ip = match current_ip() {
        Some(a) => alloc::format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]),
        None => String::from("0.0.0.0"),
    };
    let ss = scan_state();
    let scan = match ss {
        SCAN_IDLE => "idle",
        SCAN_RUNNING => "running",
        SCAN_DONE => "done",
        _ => "error",
    };
    let mut out = alloc::format!("state: {}\nssid: {}\nip: {}\nscan: {}\n", st, ssid, ip, scan);
    if ss == SCAN_DONE {
        for ap in scan_results() {
            out.push_str(&alloc::format!(
                "ap: {}\t{}\t{}\t{}\n",
                ap.ssid,
                ap.rssi,
                ap.channel,
                if ap.secured { "secure" } else { "open" }
            ));
        }
    }
    out
}

pub fn wlan_devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(WlanDevice)
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WifiStatus {
    Down,

    Connecting,

    Connected,

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

pub fn status() -> WifiStatus {
    match STATUS.load(Ordering::Acquire) {
        ST_CONNECTING => WifiStatus::Connecting,
        ST_CONNECTED => WifiStatus::Connected,
        ST_FAILED => WifiStatus::Failed,
        _ => WifiStatus::Down,
    }
}

struct NetPeripherals {
    timg0: TIMG0,
    rng: RNG,
    radio_clk: RADIO_CLK,
    wifi: WIFI,
    bt: esp_hal::peripherals::BT,
}

unsafe impl Send for NetPeripherals {}

static PENDING: Mutex<Option<NetPeripherals>> = Mutex::new(None);

pub fn provide_peripherals(
    timg0: TIMG0,
    rng: RNG,
    radio_clk: RADIO_CLK,
    wifi: WIFI,
    bt: esp_hal::peripherals::BT,
) {
    let mut g = PENDING.lock();
    *g = Some(NetPeripherals {
        timg0,
        rng,
        radio_clk,
        wifi,
        bt,
    });
}

#[inline]
fn now_smoltcp() -> Instant {
    let us = esp_hal::time::now().duration_since_epoch().to_micros();
    Instant::from_micros(us as i64)
}

/// Procesa un comando de gestión WiFi usando SÓLO el `WifiController` (nada de
/// smoltcp): así es seguro llamarlo TANTO en la fase de espera de asociación
/// (antes de montar la interfaz) como dentro del loop de servicio. Devuelve `true`
/// si hubo un (re)connect/disconnect que exige refrescar el DHCP (el llamador que
/// ya tenga smoltcp debe resetear el socket DHCP + `have_ip`).
fn process_wifi_cmd(controller: &mut WifiController<'static>, wc: WifiCmd) -> bool {
    match wc {
        WifiCmd::Scan => {
            // Para escanear, el radio NO puede estar asociado NI con un `connect()`
            // pendiente (da `EspErrWifiState`). Desconectamos SIEMPRE (aborta el
            // connect en curso y deja el STA en estado escaneable), escaneamos, y
            // reanudamos la conexión. `scan_n` se reintenta un par de veces por si
            // el `disconnect` aún no ha asentado el estado.
            *SCAN_DIAG.lock() = String::from("started");
            let _ = controller.disconnect();
            *CURRENT_IP.lock() = None;
            let t0 = uptime_ms();
            let mut attempt = 0;
            let scan = loop {
                attempt += 1;
                match controller.scan_n::<24>() {
                    Ok(v) => break Ok(v),
                    Err(e) if attempt < 3 => {
                        // Estado no listo aún (p.ej. el disconnect no ha asentado):
                        // espera ~50ms cediendo la CPU y reintenta.
                        let _ = e;
                        let until = uptime_ms().saturating_add(50);
                        while uptime_ms() < until {
                            scheduler::yield_now();
                        }
                    }
                    Err(e) => break Err(e),
                }
            };
            match scan {
                Ok((aps, _n)) => {
                    let ms = uptime_ms().saturating_sub(t0);
                    let mut out = Vec::new();
                    for ap in aps.iter() {
                        out.push(ApInfo {
                            ssid: String::from(ap.ssid.as_str()),
                            rssi: ap.signal_strength,
                            channel: ap.channel,
                            secured: !matches!(ap.auth_method, None | Some(AuthMethod::None)),
                        });
                    }
                    let found = out.len();
                    *SCAN_RESULTS.lock() = out;
                    *SCAN_DIAG.lock() = alloc::format!("{} APs in {} ms", found, ms);
                    SCAN_STATE.store(SCAN_DONE, Ordering::Release);
                }
                Err(e) => {
                    let ms = uptime_ms().saturating_sub(t0);
                    *SCAN_DIAG.lock() = alloc::format!("error {:?} after {} ms", e, ms);
                    SCAN_STATE.store(SCAN_ERROR, Ordering::Release);
                }
            }
            let _ = controller.connect();
            true
        }
        WifiCmd::Connect { ssid, password } => {
            let ssid_h = match ssid.as_str().try_into() {
                Ok(s) => s,
                Err(_) => {
                    return false;
                }
            };
            let pass_h = match password.as_str().try_into() {
                Ok(p) => p,
                Err(_) => {
                    return false;
                }
            };
            let cfg = ClientConfiguration {
                ssid: ssid_h,
                password: pass_h,
                auth_method: if password.is_empty() {
                    AuthMethod::None
                } else {
                    AuthMethod::WPA2Personal
                },
                ..Default::default()
            };
            let _ = controller.disconnect();
            if controller.set_configuration(&Configuration::Client(cfg)).is_err() {
                return false;
            }
            // Persistir en flash para que sobreviva a reinicios.
            let _ = crate::drivers::wifi_store::save(&ssid, &password);
            *CURRENT_SSID.lock() = Some(ssid.clone());
            *CURRENT_IP.lock() = None;
            set_status(WifiStatus::Connecting);
            let _ = controller.connect();
            true
        }
        WifiCmd::Disconnect => {
            let _ = controller.disconnect();
            *CURRENT_IP.lock() = None;
            set_status(WifiStatus::Down);
            true
        }
    }
}

pub fn net_task(_arg: usize) {
    set_status(WifiStatus::Down);

    let periph = { PENDING.lock().take() };
    let periph = match periph {
        Some(p) => p,
        None => {
            set_status(WifiStatus::Failed);
            return;
        }
    };

    let timg0 = TimerGroup::new(periph.timg0);
    let rng = Rng::new(periph.rng);

    let init = match esp_wifi::init(timg0.timer0, rng, periph.radio_clk) {
        Ok(c) => c,
        Err(_) => {
            set_status(WifiStatus::Failed);
            return;
        }
    };

    let init: &'static EspWifiController<'static> = Box::leak(Box::new(init));

    crate::drivers::ble::init(periph.bt, init);

    let (mut device, mut controller): (
        WifiDevice<'static, WifiStaDevice>,
        WifiController<'static>,
    ) = match wifi::new_with_mode(init, periph.wifi, WifiStaDevice) {
        Ok(v) => v,
        Err(_) => {
            set_status(WifiStatus::Failed);
            return;
        }
    };

    // Preferir credenciales GUARDADAS en flash (de un `wifi connect` anterior);
    // si no hay ninguna, usar las de compilación (wifi_credentials.rs).
    let (boot_ssid, boot_pass) = match crate::drivers::wifi_store::load() {
        Some((s, p)) => (s, p),
        None => (String::from(WIFI_SSID), String::from(WIFI_PASSWORD)),
    };

    let ssid_h = match boot_ssid.as_str().try_into() {
        Ok(s) => s,
        Err(_) => {
            set_status(WifiStatus::Failed);
            return;
        }
    };
    let pass_h = match boot_pass.as_str().try_into() {
        Ok(p) => p,
        Err(_) => {
            set_status(WifiStatus::Failed);
            return;
        }
    };
    let client = ClientConfiguration {
        ssid: ssid_h,
        password: pass_h,
        auth_method: if boot_pass.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        ..Default::default()
    };
    if controller.set_configuration(&Configuration::Client(client)).is_err() {
        set_status(WifiStatus::Failed);
        return;
    }

    if controller.start().is_err() {
        set_status(WifiStatus::Failed);
        return;
    }
    set_status(WifiStatus::Connecting);
    *CURRENT_SSID.lock() = Some(boot_ssid.clone());
    let _ = controller.connect();

    // Esperar la asociación con las credenciales de arranque ANTES de montar la
    // interfaz smoltcp. (Montar `Interface::new`/`iface.poll` sobre el dispositivo
    // esp-wifi sin asociación cuelga el driver.) Requiere credenciales válidas en
    // `wifi_credentials.rs`; para CAMBIAR de red en caliente, usa `wifi connect`
    // una vez asociado.
    // Fase de espera de asociación — SIN smoltcp montado (montar `Interface::new`/
    // `iface.poll` sobre el driver esp-wifi NO asociado lo cuelga). Reintenta
    // conectar y PROCESA los comandos `wifi` de la shell (connect/scan/disconnect)
    // hasta asociarse. Así el sistema ARRANCA aunque las credenciales de arranque
    // fallen o la red no esté; el usuario se conecta con `wifi connect`.
    let mut next_retry = uptime_ms().saturating_add(3_000);
    loop {
        let mut wcmds = Vec::new();
        {
            let mut q = WIFI_CMD_QUEUE.lock();
            wcmds.extend(q.drain(..));
        }
        for wc in wcmds {
            let _ = process_wifi_cmd(&mut controller, wc);
        }

        if matches!(controller.is_connected(), Ok(true)) {
            break;
        }
        if uptime_ms() >= next_retry {
            next_retry = uptime_ms().saturating_add(3_000);
            let _ = controller.connect();
        }
        scheduler::yield_now();
    }

    let mac = device.mac_address();
    let mut if_cfg = IfaceConfig::new(HardwareAddress::Ethernet(EthernetAddress::from_bytes(&mac)));

    if_cfg.random_seed = uptime_ms().wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut iface = Interface::new(if_cfg, &mut device, now_smoltcp());

    let mut sockets_set = SocketSet::new(Vec::new());
    let dhcp_handle = sockets_set.add(dhcpv4::Socket::new());

    let echo_rx = tcp::SocketBuffer::new(alloc::vec![0u8; ECHO_RX_SIZE]);
    let echo_tx = tcp::SocketBuffer::new(alloc::vec![0u8; ECHO_TX_SIZE]);
    let mut echo = tcp::Socket::new(echo_rx, echo_tx);
    if echo.listen(ECHO_PORT).is_err() {
        set_status(WifiStatus::Failed);
        return;
    }
    let echo_handle = sockets_set.add(echo);

    let ssh_rx = tcp::SocketBuffer::new(alloc::vec![0u8; SSH_RX_SIZE]);
    let ssh_tx = tcp::SocketBuffer::new(alloc::vec![0u8; SSH_TX_SIZE]);
    let mut ssh_sock = tcp::Socket::new(ssh_rx, ssh_tx);
    ssh_sock.set_ack_delay(None);
    if let Err(e) = ssh_sock.listen(SSH_PORT) {
        println!("[ssh] ERROR listen({}): {:?}", SSH_PORT, e);
    }
    let ssh_handle = sockets_set.add(ssh_sock);

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
    // Estado de asociación rastreado entre iteraciones: entramos aquí ya asociados
    // (el bucle de arriba esperó la conexión). Al pasar de desasociado a asociado
    // (p.ej. tras un scan o una reconexión) se pide un DHCP fresco.
    let mut associated = true;
    let mut dhcp_reset_pending = false;
    let mut next_link_check = uptime_ms().saturating_add(1_000);
    let mut buf = [0u8; ECHO_CHUNK];

    *NET_SOCKETS.lock() = Some(sockets_set);

    loop {
        {
            let t = now_smoltcp();

            let mut reconnect_dhcp = false;
            {
                let mut wcmds = Vec::new();
                {
                    let mut q = WIFI_CMD_QUEUE.lock();
                    wcmds.extend(q.drain(..));
                }
                for wc in wcmds {
                    // El helper toca sólo el controller; si hubo (re)connect/
                    // disconnect, refrescamos el DHCP y el estado de IP local.
                    if process_wifi_cmd(&mut controller, wc) {
                        reconnect_dhcp = true;
                        have_ip = false;
                    }
                }
            }

            // D-4 (BLE advertise): si el ioctl `/dev/ble0` encoló una petición, se ejecuta
            // AQUÍ, en el net_task (donde el runtime esp-wifi está activo), no síncrono en el
            // task del llamador (que colgaba). Si esto bloqueara, tumbaría el net_task
            // (wifi/SSH) — la matriz R5 lo verifica con SSH activo durante el advertise.
            crate::drivers::ble::poll_advertise();

            let mut cmds = Vec::new();
            {
                let mut q = NET_CMD_QUEUE.lock();
                cmds.extend(q.drain(..));
            }

            let mut sockets_guard = NET_SOCKETS.lock();
            let sockets = sockets_guard.as_mut().unwrap();

            if reconnect_dhcp || dhcp_reset_pending {
                iface.update_ip_addrs(|a| a.clear());
                iface.routes_mut().remove_default_ipv4_route();
                sockets.get_mut::<dhcpv4::Socket>(dhcp_handle).reset();
                dhcp_reset_pending = false;
            }

            for cmd in cmds {
                match cmd {
                    NetCmd::Connect { handle, ip, port } => {
                        let sock = sockets.get_mut::<tcp::Socket>(handle);
                        let remote_addr = smoltcp::wire::IpAddress::Ipv4(
                            smoltcp::wire::Ipv4Address::from_octets(ip),
                        );
                        let remote_endpoint = smoltcp::wire::IpEndpoint::new(remote_addr, port);
                        let local_port = 49152 + (uptime_ms() % 16384) as u16;
                        let _ = sock.connect(iface.context(), remote_endpoint, local_port);
                    }
                }
            }

            // Sólo interactuamos con el dispositivo esp-wifi (iface.poll) cuando hay
            // enlace: hacer poll/transmit sin asociación puede colgar el driver.
            if associated {
                let _ = iface.poll(t, &mut device, sockets);
            }

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
                            *CURRENT_IP.lock() = Some(ip.octets());
                            set_status(WifiStatus::Connected);
                        }
                        have_ip = true;
                    }
                }
                Some(dhcpv4::Event::Deconfigured) => {
                    iface.update_ip_addrs(|addrs| addrs.clear());
                    iface.routes_mut().remove_default_ipv4_route();
                    *CURRENT_IP.lock() = None;
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

            // Session shells are spawned from here, so they have no parent and
            // nothing can wait() for them. Sweep them up once they exit; this is
            // what actually frees a finished session's fd table and its channel.
            crate::scheduler::process::reap_orphans();

            {
                let sock = sockets.get_mut::<tcp::Socket>(ssh_handle);

                if !sock.is_open() {
                    let _ = sock.listen(SSH_PORT);
                    if ssh_active {
                        // The socket went away without a clean SSH close. Tear the
                        // session down anyway, or its shell task would sit on a
                        // channel no client can reach.
                        ssh_conn.shutdown();
                    }
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
                                ssh_conn.shutdown();
                                transport.close();
                                ssh_active = false;
                            }
                        }
                        Err(e) => {
                            if e != KError::WouldBlock {
                                println!(
                                    "[ssh] pump ERROR in state {:?}: {:?}",
                                    ssh_conn.state(),
                                    e
                                );
                                ssh_conn.shutdown();
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
                        println!("[ota] receiving image on port {}...", OTA_PORT);
                    }
                    if sock.can_recv() {
                        if let Ok(n) = sock.recv_slice(&mut buf) {
                            if n > 0 {
                                if let Err(e) = crate::ota::rx_push(&buf[..n]) {
                                    println!("[ota] ERROR while buffering: {:?}", e);
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
                            "[ota] image received: {} bytes. Flash with 'ota apply' (best from the console).",
                            total
                        );
                        sock.close();
                        ota_receiving = false;
                    }
                }
            }

            // Segunda pasada de poll para transmitir inmediatamente cualquier eco o datos
            // generados por SSH/ECHO en esta iteración sin esperar al siguiente turno de net_task.
            if associated {
                let _ = iface.poll(t, &mut device, sockets);
            }

            if uptime_ms() >= next_link_check {
                let connected = matches!(controller.is_connected(), Ok(true));
                // Chequeo frecuente (1s) mientras no hay enlace, para (re)conectar
                // rápido; espaciado (LINK_CHECK_MS) cuando ya está asociado.
                next_link_check =
                    uptime_ms().saturating_add(if connected { LINK_CHECK_MS } else { 1_000 });
                if connected {
                    if !associated {
                        // El enlace acaba de subir: pedir un DHCP fresco.
                        associated = true;
                        dhcp_reset_pending = true;
                    }
                } else {
                    if associated {
                        associated = false;
                        *CURRENT_IP.lock() = None;
                        have_ip = false;
                    }
                    set_status(WifiStatus::Connecting);
                    let _ = controller.connect();
                }
            }
        }

        scheduler::yield_now();
    }
}
