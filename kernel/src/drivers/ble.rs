#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use core::sync::atomic::{AtomicBool, Ordering};
use esp_wifi::ble::controller::BleConnector;

static ADVERTISING: Mutex<bool> = Mutex::new(false);
static CONNECTOR: Mutex<Option<BleConnector<'static>>> = Mutex::new(None);

/// D-4: petición de advertise ENCOLADA desde el ioctl. El `net_task` la consume con
/// `poll_advertise()` en su service loop — así el ioctl del llamador (p.ej. `/bin/ble`) NO
/// bloquea, y las escrituras HCI se hacen en el contexto del net_task, que es donde el
/// runtime de esp-wifi está activo. (Si aún así `conn.write` bloqueara, tumbaría el
/// net_task = wifi/SSH — por eso la matriz R5 exige verificar coexistencia, no solo `Ok`.)
static ADVERTISE_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn init(
    bt_periph: esp_hal::peripherals::BT,
    init_ref: &'static esp_wifi::EspWifiController<'static>,
) {
    let conn = BleConnector::new(init_ref, bt_periph);
    crate::arch::xtensa::interrupts::critical_section(|| {
        *CONNECTOR.lock() = Some(conn);
    });
}

pub fn start_advertising() {
    if *ADVERTISING.lock() {
        return;
    }

    // Saca el conector del Mutex para escribir los comandos HCI SIN interrupciones
    // desactivadas. El Mutex las apaga toda su vida; las escrituras al controlador BLE de
    // esp-wifi necesitan interrupciones/timer para completar, así que hacerlas BAJO el lock
    // cuelga la placa (lección D-12: ningún I/O bajo lock — el mismo error que el VFS ya
    // corrigió). Se saca (lock breve), se escribe desbloqueado, y se devuelve.
    let mut conn = match CONNECTOR.lock().take() {
        Some(c) => c,
        None => {
            esp_println::println!("[ble] ERROR: BLE controller not initialized");
            return;
        }
    };

    use embedded_io::Write;

    let params: [u8; 19] = [
        0x01, 0x06, 0x20, 0x0f, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x07, 0x00,
    ];
    let _ = conn.write(&params);

    let mut data: [u8; 36] = [0; 36];
    data[0] = 0x01;
    data[1] = 0x08;
    data[2] = 0x20;
    data[3] = 32;
    data[4] = 14;

    data[5] = 2;
    data[6] = 0x01;
    data[7] = 0x06;

    data[8] = 11;
    data[9] = 0x09;
    let name = b"EspressoOS";
    data[10..20].copy_from_slice(name);

    let _ = conn.write(&data);

    let enable: [u8; 5] = [0x01, 0x0a, 0x20, 0x01, 0x01];
    let _ = conn.write(&enable);

    // Devuelve el conector al Mutex y marca el estado (locks breves, sin I/O bajo ellos).
    *CONNECTOR.lock() = Some(conn);
    *ADVERTISING.lock() = true;
    esp_println::println!("[ble] BLE advertising started as 'EspressoOS'");
}

pub fn is_advertising() -> bool {
    *ADVERTISING.lock()
}

/// Encola una petición de advertise (NO bloquea al llamador). La procesa el net_task.
pub fn request_advertise() {
    ADVERTISE_REQUESTED.store(true, Ordering::Release);
}

/// Llamado por el `net_task` en cada iteración de su service loop: si hay una petición de
/// advertise pendiente, la ejecuta. Las escrituras HCI (`start_advertising`) ocurren AQUÍ,
/// en el contexto del net_task donde el runtime de esp-wifi está activo.
pub fn poll_advertise() {
    if ADVERTISE_REQUESTED.swap(false, Ordering::AcqRel) {
        start_advertising();
    }
}

// ---- /dev/ble0: estado por read (D-3), advertise por ioctl (SP2 R5). ----

pub const BLE_ADVERTISE: u32 = 0;
/// DIAGNÓSTICO temporal (experimento de hipótesis de pila). Solo existe bajo la
/// feature `diag-ble-sync`; el build por defecto NO lo compila (0xD1A6 cae en
/// InvalidArgument). QUITAR el código tras cerrar el experimento.
#[cfg(feature = "diag-ble-sync")]
pub const BLE_ADVERTISE_SYNC_DIAG: u32 = 0xD1A6;

struct BleDevice;

impl crate::vfs::devfs::Device for BleDevice {
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let s = if is_advertising() {
            "state: advertising as 'EspressoOS'\n"
        } else {
            "state: inactive\n"
        };
        let bytes = s.as_bytes();
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
    fn ioctl(&self, cmd: u32, _arg: usize) -> KResult<usize> {
        match cmd {
            BLE_ADVERTISE => {
                // D-4: ENCOLA (no bloquea al llamador); el net_task ejecuta las escrituras
                // HCI en su contexto (poll_advertise). Ya NO llama a start_advertising
                // síncrono, que colgaba el board (I/O bloqueante fuera del runtime).
                request_advertise();
                Ok(0)
            }
            #[cfg(feature = "diag-ble-sync")]
            BLE_ADVERTISE_SYNC_DIAG => {
                // DIAGNÓSTICO (hipótesis: desbordamiento de la pila de userland por los
                // frames C de NimBLE encima del syscall). Corre start_advertising SÍNCRONO
                // en la pila del LLAMADOR (/bin/ble, 16K) — el camino que colgaba. El pico
                // de pila se mide con stacks_report (escanea la pintura 0xDEADBEEF, que no
                // se restaura al retornar los frames). Si a 16K cuelga y a 32K sobrevive
                // con used>16K → mecanismo probado. QUITAR tras cerrar el experimento.
                esp_println::println!("[diag] BEFORE (baseline, pila del llamador):");
                esp_println::println!("{}", crate::scheduler::stacks_report());
                *ADVERTISING.lock() = false; // fuerza el camino profundo aunque ya anuncie
                esp_println::println!("[diag] llamando start_advertising SINCRONO...");
                start_advertising();
                esp_println::println!("[diag] SOBREVIVIO. AFTER (pico):");
                esp_println::println!("{}", crate::scheduler::stacks_report());
                Ok(0)
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(BleDevice)
}
