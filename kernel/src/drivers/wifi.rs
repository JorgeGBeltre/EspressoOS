//! Controlador WiFi: binding de `esp-wifi` (radio 802.11) + `smoltcp` (TCP/IP).
//!
//! Este módulo une tres piezas:
//!   1. La radio 802.11 y su firmware, gestionados por `esp-wifi`.
//!   2. El stack TCP/IP `no_std` `smoltcp`, alimentado por el `WifiDevice` que
//!      expone `esp-wifi` (implementa `smoltcp::phy::Device`).
//!   3. Un cliente DHCP (socket `dhcpv4`) para obtener IP automáticamente.
//!
//! El resto del kernel solo ve la API canónica del contrato (§3.9):
//!   `init`, `connect`, `disconnect`, `status` (+ `WifiStatus`), más los extras
//!   `poll()` y el helper de socket TCP cliente (`tcp_connect`/`tcp_send`/...).
//!
//! ============================ AVISO DE RIESGO ============================
//! ESTE ARCHIVO ES DE ALTO RIESGO Y NO SE HA PODIDO COMPILAR NI PROBAR CONTRA
//! HARDWARE. La superficie de API de `esp-wifi` y `smoltcp` es de las más
//! volátiles del ecosistema esp-rs y cambia entre versiones menores. Cada punto
//! donde la firma exacta no está 100% confirmada está marcado con `// (?)`.
//! El implementador con la toolchain instalada DEBE verificar esos puntos
//! contra los crates realmente resueltos (ver `needs_crates`).
//! ========================================================================
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use crate::prelude::*;

// Reloj monotónico del kernel: smoltcp necesita marcas de tiempo en ms.
use crate::arch::xtensa::timer::uptime_ms;
// Mutex canónico del kernel (§3.2.4). Protege TODO el estado global de red.
use crate::arch::xtensa::sync::Mutex;

use core::sync::atomic::{AtomicU8, AtomicU16, Ordering};

// -- Periféricos de esp-hal que consume el init real (variante con periférico) --
use esp_hal::peripherals::{RADIO_CLK, RNG, TIMG0, WIFI};
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;

// -- API de esp-wifi (0.12.x, coherente con esp-hal 0.23.x). (?) toda la ruta. --
use esp_wifi::wifi::{
    AuthMethod, ClientConfiguration, Configuration, WifiController, WifiDevice, WifiStaDevice,
};
use esp_wifi::EspWifiController;

// -- API de smoltcp (0.12.x). (?) rutas y firmas exactas. --
use smoltcp::iface::{Config as IfaceConfig, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr};

// ============================================================================
// Estado del enlace — visible por `status()`.
// ============================================================================

/// Estado del enlace WiFi (contrato §3.9). No cambiar las variantes.
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

// Codificación compacta del estado en un átomo para que `status()` sea
// consultable sin tomar el Mutex (útil desde cualquier contexto, incluso ISR).
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

/// Estado actual del enlace (contrato §3.9). Lectura sin bloqueo.
pub fn status() -> WifiStatus {
    match STATUS.load(Ordering::Acquire) {
        ST_CONNECTING => WifiStatus::Connecting,
        ST_CONNECTED => WifiStatus::Connected,
        ST_FAILED => WifiStatus::Failed,
        _ => WifiStatus::Down,
    }
}

// ============================================================================
// Parámetros de configuración del stack.
// ============================================================================

/// Tamaño de los búferes RX/TX de cada socket TCP (bytes). Ajustable.
const TCP_BUF_SIZE: usize = 1536;
/// Tiempo máximo esperando asociación al AP (ms).
const ASSOC_TIMEOUT_MS: u64 = 15_000;
/// Tiempo máximo esperando IP por DHCP tras asociarse (ms).
const DHCP_TIMEOUT_MS: u64 = 15_000;
/// Primer puerto efímero para clientes TCP salientes.
const EPHEMERAL_PORT_BASE: u16 = 49_152;

/// Contador de puertos efímeros (rota en el rango 49152..=65535).
static NEXT_PORT: AtomicU16 = AtomicU16::new(EPHEMERAL_PORT_BASE);

fn next_ephemeral_port() -> u16 {
    // fetch_add con wrap manual al techo del rango dinámico/privado.
    let p = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    if p >= 65_000 {
        NEXT_PORT.store(EPHEMERAL_PORT_BASE, Ordering::Relaxed);
    }
    p
}

// ============================================================================
// Estado global de red.
// ============================================================================

/// Agrupa todo el estado vivo del subsistema de red. Vive en un único
/// `static Mutex<Option<WifiState>>` (regla §6.7: nada de `static mut`).
///
/// Nota de lifetimes: `esp-wifi` entrega `WifiDevice`/`WifiController` con un
/// lifetime atado al `EspWifiController`. Para poder almacenarlos en un `static`
/// se "fuga" (`Box::leak`) el `EspWifiController`, obteniendo referencias
/// `'static`. Es el patrón estándar (equivalente al macro `mk_static!` de los
/// ejemplos de esp-wifi) y es seguro porque ese objeto vive lo que dura el SO.
struct WifiState {
    /// Controlador de la radio: set_configuration/start/connect/disconnect.
    controller: WifiController<'static>,
    /// Dispositivo de enlace (capa 2) que consume smoltcp en cada `poll`.
    device: WifiDevice<'static, WifiStaDevice>,
    /// Interfaz IP de smoltcp (mantiene direcciones, rutas, ARP/ND, etc.).
    iface: Interface,
    /// Conjunto de sockets activos (TCP cliente + DHCP).
    sockets: SocketSet<'static>,
    /// Handle del socket DHCPv4 (siempre presente tras `init_hw`).
    dhcp: SocketHandle,
}

// SEGURIDAD: en Fase 7 el SO es monociclo (un solo core planificando red) y todo
// acceso a `WifiState` pasa por el `Mutex`. Los tipos de esp-wifi/smoltcp no son
// `Send` por construcción; marcamos el contenedor como `Send` bajo esa premisa.
// Cuando llegue SMP (Fase 9) hay que revisar la afinidad de la pila de red.
unsafe impl Send for WifiState {}

/// Estado global protegido. `None` hasta que `init_hw` complete la bring-up.
static NET: Mutex<Option<WifiState>> = Mutex::new(None);

// ============================================================================
// Inicialización.
// ============================================================================

/// Inicializa el subsistema WiFi (contrato §3.9).
///
/// PROBLEMA DE PROPIEDAD: `esp-wifi` necesita tomar posesión de varios
/// periféricos (`WIFI`, `RADIO_CLK`, un timer y `RNG`) que solo `main` posee tras
/// `esp_hal::init`. Como la firma canónica del contrato no lleva periféricos
/// (§3.9 / §5), la bring-up REAL se hace en [`init_hw`], que `main` invoca con los
/// campos de `peripherals` (§5 nota "Propiedad de `peripherals`").
///
/// Esta función canónica se limita a comprobar que la bring-up ya ocurrió:
///   * `Ok(())`                 -> `init_hw` corrió y el stack está listo.
///   * `Err(KError::NotSupported)` -> aún no se han cedido los periféricos.
pub fn init() -> KResult<()> {
    let guard = NET.lock();
    if guard.is_some() {
        Ok(())
    } else {
        // El integrador debe llamar antes a `wifi::init_hw(...)` desde `main`.
        Err(KError::NotSupported)
    }
}

/// Bring-up REAL de la radio + stack IP (variante con periférico de `init`).
///
/// Se llama UNA vez desde `main` (paso §5.14), p. ej.:
/// ```ignore
/// drivers::wifi::init_hw(
///     peripherals.TIMG0,
///     peripherals.RNG,
///     peripherals.RADIO_CLK,
///     peripherals.WIFI,
/// )?;
/// ```
/// Requiere que el heap ya esté inicializado (`mm::heap::init`): `esp-wifi`
/// asigna sus estructuras internas en el allocator global.
///
/// (?) La firma de `esp_wifi::init` y `new_with_mode` cambia entre versiones;
/// verificar contra el crate resuelto (ver `needs_crates`).
pub fn init_hw(
    timg0: TIMG0,
    rng_periph: RNG,
    radio_clk: RADIO_CLK,
    wifi: WIFI,
) -> KResult<()> {
    // -- 1. Fuentes de tiempo y aleatoriedad que exige el firmware WiFi. --
    // esp-wifi usa un timer (aquí TIMG0.timer0) para su planificador interno y
    // un RNG para la entropía del stack de seguridad WPA.
    let timg0 = TimerGroup::new(timg0);
    let rng = Rng::new(rng_periph);

    // -- 2. Inicializar el firmware de radio. --
    // (?) En 0.12.x: `esp_wifi::init(timer, rng, radio_clk) -> Result<EspWifiController, _>`.
    //     Versiones previas llevaban un `EspWifiInitFor::Wifi` extra.
    let controller_fw = esp_wifi::init(timg0.timer0, rng, radio_clk)
        .map_err(|_| KError::IoError)?;

    // Fugamos el `EspWifiController` para obtener referencias `'static` (patrón
    // `mk_static!`). Vive lo que dura el SO; no se libera nunca.
    let controller_fw: &'static EspWifiController<'static> =
        Box::leak(Box::new(controller_fw));

    // -- 3. Crear el par (dispositivo de enlace, controlador) en modo estación. --
    // (?) `new_with_mode(&ctrl, WIFI, WifiStaDevice) -> (WifiDevice, WifiController)`.
    let (device, controller): (WifiDevice<'static, WifiStaDevice>, WifiController<'static>) =
        esp_wifi::wifi::new_with_mode(controller_fw, wifi, WifiStaDevice)
            .map_err(|_| KError::IoError)?;
    let mut device = device;

    // -- 4. Construir la interfaz IP de smoltcp sobre el dispositivo. --
    // Dirección MAC: la provee el dispositivo. (?) nombre exacto `mac_address()`.
    // Si no existiese, usar una MAC localmente administrada de respaldo.
    let mac = device.mac_address();
    let hw = HardwareAddress::Ethernet(EthernetAddress(mac));

    let mut if_cfg = IfaceConfig::new(hw);
    // Semilla aleatoria para puertos/secuencias TCP (mejora robustez). (?)
    if_cfg.random_seed = uptime_ms().wrapping_mul(0x9E37_79B9_7F4A_7C15);

    let now = Instant::from_millis(uptime_ms() as i64);
    let iface = Interface::new(if_cfg, &mut device, now);

    // -- 5. Conjunto de sockets (almacenamiento propio en heap vía `alloc`). --
    let mut sockets = SocketSet::new(Vec::new());

    // Socket DHCPv4: obtendrá IP/gateway/DNS automáticamente al conectar.
    let dhcp_socket = dhcpv4::Socket::new();
    let dhcp = sockets.add(dhcp_socket);

    // -- 6. Publicar el estado global. --
    let state = WifiState {
        controller,
        device,
        iface,
        sockets,
        dhcp,
    };
    let mut guard = NET.lock();
    *guard = Some(state);
    set_status(WifiStatus::Down);
    Ok(())
}

// ============================================================================
// Asociación al AP.
// ============================================================================

/// Conecta a la red `ssid` con `password` (contrato §3.9).
///
/// Flujo: configura modo estación -> `start()` -> `connect()` -> espera
/// asociación -> lanza DHCP y espera IP. Bloqueante con tiempos de guarda.
/// NUNCA panica; ante fallo deja `status()==Failed` y devuelve `KError`.
///
/// IMPORTANTE sobre el bloqueo: no mantenemos el `Mutex` de `NET` tomado durante
/// las esperas; lo tomamos y soltamos en cada iteración. El firmware de esp-wifi
/// avanza su máquina de estados por interrupciones/tareas propias, ajenas a
/// nuestro lock, así que soltar entre sondeos evita bloqueos e inanición.
pub fn connect(ssid: &str, password: &str) -> KResult<()> {
    if ssid.is_empty() {
        return Err(KError::InvalidArgument);
    }

    set_status(WifiStatus::Connecting);

    // -- 1. Configurar y arrancar la radio (sección corta con el lock tomado). --
    {
        let mut guard = NET.lock();
        let st = guard.as_mut().ok_or(KError::NotSupported)?;

        // `ssid`/`password` de esp-wifi son `heapless::String<32>`/`<64>`.
        // Convertimos con `try_into` (no metemos `heapless` en las firmas).
        // Ante fallo se respeta la postcondición documentada: `status()==Failed`.
        let ssid_h = ssid.try_into().map_err(|_| {
            set_status(WifiStatus::Failed);
            KError::NameTooLong
        })?;
        let pass_h = password.try_into().map_err(|_| {
            set_status(WifiStatus::Failed);
            KError::NameTooLong
        })?;

        let client = ClientConfiguration {
            ssid: ssid_h,
            password: pass_h,
            // Red abierta si no hay contraseña; si no, WPA2 personal por defecto.
            auth_method: if password.is_empty() {
                AuthMethod::None
            } else {
                AuthMethod::WPA2Personal
            },
            ..Default::default()
        };

        st.controller
            .set_configuration(&Configuration::Client(client))
            .map_err(|_| {
                set_status(WifiStatus::Failed);
                KError::InvalidArgument
            })?;

        // `start()` arranca el driver; idempotente si ya estaba iniciado. (?)
        // Ignoramos el error "ya iniciado" tratándolo como no fatal.
        let _ = st.controller.start();

        // Reiniciar el cliente DHCP para forzar un DISCOVER limpio. (?)
        // `st.dhcp` es un handle SIEMPRE válido (creado en `init_hw`), por lo que
        // `get_mut` no puede fallar aquí: no viola la prohibición de panics.
        st.sockets.get_mut::<dhcpv4::Socket>(st.dhcp).reset();

        // Lanzar la asociación. (?) `connect()` bloqueante en 0.12.x.
        st.controller.connect().map_err(|_| {
            set_status(WifiStatus::Failed);
            KError::IoError
        })?;
    }

    // -- 2. Esperar asociación a nivel de enlace (con timeout). --
    let assoc_deadline = uptime_ms().saturating_add(ASSOC_TIMEOUT_MS);
    loop {
        // Sondeo puntual del estado de conexión de la radio.
        let connected = {
            let mut guard = NET.lock();
            let st = guard.as_mut().ok_or(KError::NotSupported)?;
            // (?) `is_connected() -> Result<bool, WifiError>` en 0.12.x.
            st.controller.is_connected().unwrap_or(false)
        };
        if connected {
            break;
        }
        if uptime_ms() >= assoc_deadline {
            set_status(WifiStatus::Failed);
            return Err(KError::Timeout);
        }
        // Espera activa breve. Sin `Delay` aquí para no depender del scheduler;
        // el firmware progresa por su cuenta entre sondeos.
        spin_hint();
    }

    // -- 3. DHCP: bombear el stack hasta obtener IPv4 (con timeout). --
    let dhcp_deadline = uptime_ms().saturating_add(DHCP_TIMEOUT_MS);
    loop {
        // `poll()` procesa tramas entrantes/salientes y eventos DHCP.
        let _ = poll();

        let has_ip = {
            let mut guard = NET.lock();
            let st = guard.as_mut().ok_or(KError::NotSupported)?;
            // (?) `Interface::ipv4_addr() -> Option<Ipv4Address>`.
            st.iface.ipv4_addr().is_some()
        };
        if has_ip {
            set_status(WifiStatus::Connected);
            return Ok(());
        }
        if uptime_ms() >= dhcp_deadline {
            set_status(WifiStatus::Failed);
            return Err(KError::Timeout);
        }
        spin_hint();
    }
}

/// Desconecta del AP y baja el enlace (contrato §3.9).
pub fn disconnect() -> KResult<()> {
    let mut guard = NET.lock();
    let st = guard.as_mut().ok_or(KError::NotSupported)?;
    // (?) `disconnect()` bloqueante en 0.12.x.
    let _ = st.controller.disconnect();
    set_status(WifiStatus::Down);
    Ok(())
}

// ============================================================================
// Bombeo del stack (poll) — EXTRA solicitado.
// ============================================================================

/// Avanza el stack de red: procesa tramas entrantes/salientes y eventos DHCP.
///
/// Debe llamarse con frecuencia (p. ej. desde una tarea de red o tras cada
/// operación de socket). Es no-bloqueante: hace una pasada y retorna.
/// Devuelve `Ok(())` siempre que el subsistema esté inicializado.
pub fn poll() -> KResult<()> {
    let mut guard = NET.lock();
    let st = guard.as_mut().ok_or(KError::NotSupported)?;

    let now = Instant::from_millis(uptime_ms() as i64);

    // Una pasada del motor smoltcp. (?) firma `poll(now, &mut device, &mut sockets)`.
    let _ = st.iface.poll(now, &mut st.device, &mut st.sockets);

    // Procesar el resultado del cliente DHCP y aplicar la configuración IP.
    // (?) `dhcpv4::Socket::poll() -> Option<dhcpv4::Event>`.
    // `st.dhcp` es válido por construcción -> `get_mut` no panica. Extraemos el
    // evento (valor propio) para soltar el préstamo de `sockets` antes de tocar
    // `iface` en las ramas (préstamos disjuntos, sin solapamiento).
    let event = st.sockets.get_mut::<dhcpv4::Socket>(st.dhcp).poll();
    match event {
        Some(dhcpv4::Event::Configured(config)) => {
            // Dirección + prefijo asignados por el servidor DHCP.
            st.iface.update_ip_addrs(|addrs| {
                addrs.clear();
                // `config.address` es un `Ipv4Cidr`.
                let _ = addrs.push(IpCidr::Ipv4(config.address));
            });
            // Ruta por defecto hacia el gateway, si lo hay.
            if let Some(router) = config.router {
                let _ = st.iface.routes_mut().add_default_ipv4_route(router);
            }
            // (Los servidores DNS de `config.dns_servers` se guardarían aquí
            //  cuando se implemente el resolver — Fase 7, servicio DNS.)
        }
        Some(dhcpv4::Event::Deconfigured) => {
            // Perdimos el lease: limpiar IP y ruta por defecto.
            st.iface.update_ip_addrs(|addrs| addrs.clear());
            st.iface.routes_mut().remove_default_ipv4_route();
            if status() == WifiStatus::Connected {
                set_status(WifiStatus::Connecting);
            }
        }
        None => {}
    }

    Ok(())
}

// ============================================================================
// Helper de socket TCP cliente — EXTRA solicitado.
// ============================================================================

/// Asa a un socket TCP gestionado por el stack global. Envuelve el
/// `SocketHandle` de smoltcp para no filtrar el tipo a los llamadores.
#[derive(Clone, Copy)]
pub struct TcpSocket {
    handle: SocketHandle,
    /// Puerto local efímero asignado (informativo/diagnóstico).
    local_port: u16,
}

/// Abre un socket TCP y lanza la conexión a `ip:port` (IPv4).
///
/// No bloquea hasta completar el 3-way handshake; devuelve el asa en cuanto la
/// conexión queda "en curso". El llamador debe bombear con [`poll`] y consultar
/// [`tcp_is_connected`] antes de enviar/recibir.
///
/// `ip` en orden de red (a.b.c.d). Devuelve `Busy` si no hay memoria de socket.
pub fn tcp_connect(ip: [u8; 4], port: u16) -> KResult<TcpSocket> {
    if port == 0 {
        return Err(KError::InvalidArgument);
    }

    let mut guard = NET.lock();
    let st = guard.as_mut().ok_or(KError::NotSupported)?;

    // Búferes RX/TX propios (heap). smoltcp toma posesión de ellos.
    let rx = tcp::SocketBuffer::new(alloc::vec![0u8; TCP_BUF_SIZE]);
    let tx = tcp::SocketBuffer::new(alloc::vec![0u8; TCP_BUF_SIZE]);
    let socket = tcp::Socket::new(rx, tx);
    let handle = st.sockets.add(socket);

    let local_port = next_ephemeral_port();
    // Destino: IpAddress::Ipv4. En smoltcp 0.12 las direcciones usan `core::net`.
    // (?) construcción exacta; alternativa histórica: `Ipv4Address::new(...)`.
    let remote = IpAddress::Ipv4(core::net::Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]));

    // `connect` necesita el contexto de la interfaz para elegir ruta/IP local.
    // Aislamos referencias mutables a campos distintos del mismo struct.
    let WifiState {
        ref mut iface,
        ref mut sockets,
        ..
    } = *st;
    let sock = sockets.get_mut::<tcp::Socket>(handle);
    // Si `connect` falla, RETIRAR el socket recién añadido para no fugar su
    // ranura ni sus búferes RX/TX (heap). El préstamo de `sock` termina en la
    // llamada, así que `sockets.remove` es válido aquí.
    if sock
        .connect(iface.context(), (remote, port), local_port)
        .is_err()
    {
        sockets.remove(handle);
        return Err(KError::IoError);
    }

    Ok(TcpSocket { handle, local_port })
}

/// Indica si la conexión TCP está establecida (handshake completo).
pub fn tcp_is_connected(sock: &TcpSocket) -> KResult<bool> {
    let mut guard = NET.lock();
    let st = guard.as_mut().ok_or(KError::NotSupported)?;
    let s = st.sockets.get_mut::<tcp::Socket>(sock.handle);
    // `may_send() && may_recv()` aproxima "conectado y utilizable". (?)
    Ok(s.is_active() && s.may_send() && s.may_recv())
}

/// Envía datos por el socket (no-bloqueante). Devuelve bytes encolados.
///
/// Solo copia lo que quepa en el búfer TX; el envío real ocurre en [`poll`].
/// Si el socket aún no puede enviar, devuelve `Ok(0)` (reintentar tras `poll`).
pub fn tcp_send(sock: &TcpSocket, data: &[u8]) -> KResult<usize> {
    let sent = {
        let mut guard = NET.lock();
        let st = guard.as_mut().ok_or(KError::NotSupported)?;
        let s = st.sockets.get_mut::<tcp::Socket>(sock.handle);
        if !s.can_send() {
            0
        } else {
            s.send_slice(data).map_err(|_| KError::IoError)?
        }
    };
    // Empujar de inmediato lo encolado hacia la red.
    let _ = poll();
    Ok(sent)
}

/// Recibe datos del socket (no-bloqueante). Devuelve bytes copiados en `buf`.
///
/// `Ok(0)` significa "sin datos por ahora" (reintentar tras [`poll`]).
pub fn tcp_recv(sock: &TcpSocket, buf: &mut [u8]) -> KResult<usize> {
    // Procesar entrantes antes de leer.
    let _ = poll();
    let mut guard = NET.lock();
    let st = guard.as_mut().ok_or(KError::NotSupported)?;
    let s = st.sockets.get_mut::<tcp::Socket>(sock.handle);
    if !s.can_recv() {
        return Ok(0);
    }
    let n = s.recv_slice(buf).map_err(|_| KError::IoError)?;
    Ok(n)
}

/// Cierra ordenadamente el socket (envía FIN) y libera su ranura.
///
/// Tras el cierre lógico se bombea una vez para cursar el FIN; luego se elimina
/// del conjunto de sockets (liberando sus búferes).
pub fn tcp_close(sock: &TcpSocket) -> KResult<()> {
    {
        let mut guard = NET.lock();
        let st = guard.as_mut().ok_or(KError::NotSupported)?;
        let s = st.sockets.get_mut::<tcp::Socket>(sock.handle);
        s.close();
    }
    let _ = poll();
    // Retirar el socket del set para no fugar sus búferes.
    let mut guard = NET.lock();
    let st = guard.as_mut().ok_or(KError::NotSupported)?;
    st.sockets.remove(sock.handle);
    Ok(())
}

// ============================================================================
// Utilidades internas.
// ============================================================================

/// Pista de espera activa para bucles de sondeo. Barata y sin dormir.
#[inline]
fn spin_hint() {
    core::hint::spin_loop();
}

// ----------------------------------------------------------------------------
// NOTA (?) sobre `SocketSet::get_mut`: en smoltcp `get_mut::<T>(handle)` PANICA
// si el handle es inválido o el tipo no coincide. Aquí solo se invoca sobre
// handles garantizados por construcción (`st.dhcp` creado en `init_hw`; los de
// TCP recién añadidos en `tcp_connect`), por lo que no puede panicar en la
// práctica. Aun así, como el kernel PROHÍBE panics en sus rutas, el implementador
// con toolchain debería, si la versión resuelta lo ofrece, migrar a un accesor no
// panicante (p. ej. `try_get_mut`) para blindar el invariante ante refactors.
//
// OTROS PUNTOS A VERIFICAR CONTRA LOS CRATES RESUELTOS (todos marcados `(?)`):
//   * `esp_wifi::init` / `new_with_mode`: aridad y tipos de retorno.
//   * `WifiController::{set_configuration,start,connect,disconnect,is_connected}`.
//   * `WifiDevice::mac_address`.
//   * `Interface::{poll,ipv4_addr,update_ip_addrs,routes_mut,context}` de smoltcp.
//   * Construcción de `IpAddress::Ipv4` con `core::net::Ipv4Addr` (smoltcp 0.12
//     migró a `core::net`; versiones previas usaban su propio `Ipv4Address`).
//   * `esp-hal` probablemente necesite su feature `unstable` habilitada para que
//     `esp-wifi` compile (timers/RNG usados por el firmware). Eso lo ajusta el
//     agente de integración en `kernel/Cargo.toml` (fuera del alcance de este
//     archivo).
// ----------------------------------------------------------------------------
