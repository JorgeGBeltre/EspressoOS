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


pub static CURRENT_IP: Mutex<Option<[u8; 4]>> = Mutex::new(None);
pub static CURRENT_SSID: Mutex<Option<String>> = Mutex::new(None);



pub fn request_scan() {
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
    WIFI_CMD_QUEUE.lock().push(WifiCmd::Connect { ssid, password });
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









pub fn provide_peripherals(timg0: TIMG0, rng: RNG, radio_clk: RADIO_CLK, wifi: WIFI, bt: esp_hal::peripherals::BT) {
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









pub fn net_task(_arg: usize) {
    set_status(WifiStatus::Down);


    let periph = { PENDING.lock().take() };
    let periph = match periph {
        Some(p) => p,
        None => {
            println!("[net] ERROR: peripherals not provided (was provide_peripherals missing?)");
            set_status(WifiStatus::Failed);
            return;
        }
    };





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


    let init: &'static EspWifiController<'static> = Box::leak(Box::new(init));

    crate::drivers::ble::init(periph.bt, init);

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
            println!("[net] ERROR: SSID too long (>32)");
            set_status(WifiStatus::Failed);
            return;
        }
    };
    let pass_h = match WIFI_PASSWORD.try_into() {
        Ok(p) => p,
        Err(_) => {
            println!("[net] ERROR: password too long (>64)");
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
    *CURRENT_SSID.lock() = Some(String::from(WIFI_SSID));
    println!("[net] connecting to SSID '{}'...", WIFI_SSID);
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
    println!("[net] associated with AP; negotiating DHCP...");

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




            let mut reconnect_dhcp = false;
            {
                let mut wcmds = Vec::new();
                {
                    let mut q = WIFI_CMD_QUEUE.lock();
                    wcmds.extend(q.drain(..));
                }
                for wc in wcmds {
                    match wc {
                        WifiCmd::Scan => match controller.scan_n::<24>() {
                            Ok((aps, _)) => {
                                let mut out = Vec::new();
                                for ap in aps.iter() {
                                    out.push(ApInfo {
                                        ssid: String::from(ap.ssid.as_str()),
                                        rssi: ap.signal_strength,
                                        channel: ap.channel,
                                        secured: !matches!(
                                            ap.auth_method,
                                            None | Some(AuthMethod::None)
                                        ),
                                    });
                                }
                                *SCAN_RESULTS.lock() = out;
                                SCAN_STATE.store(SCAN_DONE, Ordering::Release);
                            }
                            Err(e) => {
                                println!("[net] scan error: {:?}", e);
                                SCAN_STATE.store(SCAN_ERROR, Ordering::Release);
                            }
                        },
                        WifiCmd::Connect { ssid, password } => {
                            let ssid_h = match ssid.as_str().try_into() {
                                Ok(s) => s,
                                Err(_) => {
                                    println!("[net] ERROR: SSID too long (>32)");
                                    continue;
                                }
                            };
                            let pass_h = match password.as_str().try_into() {
                                Ok(p) => p,
                                Err(_) => {
                                    println!("[net] ERROR: password too long (>64)");
                                    continue;
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
                            if let Err(e) =
                                controller.set_configuration(&Configuration::Client(cfg))
                            {
                                println!("[net] ERROR set_configuration: {:?}", e);
                                continue;
                            }
                            *CURRENT_SSID.lock() = Some(ssid.clone());
                            *CURRENT_IP.lock() = None;
                            have_ip = false;
                            reconnect_dhcp = true;
                            set_status(WifiStatus::Connecting);
                            println!("[net] switching to SSID '{}'...", ssid);
                            let _ = controller.connect();
                        }
                        WifiCmd::Disconnect => {
                            let _ = controller.disconnect();
                            *CURRENT_IP.lock() = None;
                            have_ip = false;
                            reconnect_dhcp = true;
                            set_status(WifiStatus::Down);
                            println!("[net] disconnected by user");
                        }
                    }
                }
            }


            let mut cmds = Vec::new();
            {
                let mut q = NET_CMD_QUEUE.lock();
                cmds.extend(q.drain(..));
            }

            let mut sockets_guard = NET_SOCKETS.lock();
            let sockets = sockets_guard.as_mut().unwrap();




            if reconnect_dhcp {
                iface.update_ip_addrs(|a| a.clear());
                iface.routes_mut().remove_default_ipv4_route();
                sockets.get_mut::<dhcpv4::Socket>(dhcp_handle).reset();
            }

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
                            *CURRENT_IP.lock() = Some(ip.octets());
                            println!("[net] IP = {}", ip);
                            println!(
                                "[net] SSH listening on port {}, ECHO on {}, OTA on {}",
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
                                println!("[ssh] pump ERROR in state {:?}: {:?}", ssh_conn.state(), e);
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
        }

        scheduler::yield_now();
    }
}

