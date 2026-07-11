//! Driver de consola: USB-Serial-JTAG del ESP32-S3 (base de `/dev/console`).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! En Fase 0 la salida "bonita" del banner la produce `esp-println`. Este módulo
//! implementa el driver PROPIO de consola que usará el kernel a través del VFS
//! (`vfs::devfs`) como `/dev/console`:
//!
//!   * TX con un pequeño buffer software (anillo) que se vuelca al FIFO del
//!     endpoint IN del periférico USB-Serial-JTAG.
//!   * RX no bloqueante leyendo el FIFO del endpoint OUT.
//!
//! Se accede por registros del periférico USB_SERIAL_JTAG (0x6003_8000). Es el
//! camino más portable en la placa DevKit (consola por el conector USB nativo).
//! Alternativa UART0 (pines TX/RX físicos): ver notas marcadas [HW] más abajo.
//!
//! # Notas de validación en hardware [HW]
//! * Los offsets de EP1_REG / EP1_CONF_REG y el significado de sus bits están
//!   tomados del TRM del S3; confirmar contra el chip real.
//! * El protocolo exacto de "flush" (bit WR_DONE) y la latencia de
//!   `IN_EP_DATA_FREE` dependen del host USB; ajustar `MAX_SPIN` si hiciera falta.
//! * Si el host USB no está conectado, el FIFO IN puede no vaciarse nunca: por
//!   eso el volcado tiene un límite de reintentos y NUNCA bloquea el kernel.
#![allow(dead_code)]

use crate::arch::xtensa::sync::SpinLock;
use crate::prelude::*;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// Mapa de registros del periférico USB_SERIAL_JTAG (ESP32-S3). [HW]
// ---------------------------------------------------------------------------

/// Base del periférico USB_SERIAL_JTAG.
const USB_SERIAL_JTAG_BASE: usize = 0x6003_8000;
/// FIFO de datos del endpoint (RDWR_BYTE): escribir = cargar TX, leer = extraer RX.
const EP1_REG: usize = USB_SERIAL_JTAG_BASE + 0x0000;
/// Registro de configuración/estado del endpoint 1.
const EP1_CONF_REG: usize = USB_SERIAL_JTAG_BASE + 0x0004;

/// Bit 0 (WR_DONE): al escribir 1 se entrega ("flush") el paquete TX al host.
const CONF_WR_DONE: u32 = 1 << 0;
/// Bit 1 (SERIAL_IN_EP_DATA_FREE): 1 = el FIFO del endpoint IN admite más bytes.
const CONF_IN_EP_DATA_FREE: u32 = 1 << 1;
/// Bit 2 (SERIAL_OUT_EP_DATA_AVAIL): 1 = hay bytes disponibles para leer (RX).
const CONF_OUT_EP_DATA_AVAIL: u32 = 1 << 2;

/// Tope de reintentos al esperar hueco en el FIFO IN. Si el host no drena
/// (p. ej. USB desconectado) abandonamos el volcado para no colgar el kernel. [HW]
const MAX_SPIN: u32 = 100_000;

/// Capacidad del anillo de transmisión (bytes). Suficiente para líneas de log.
const TX_BUF_LEN: usize = 512;

// ---------------------------------------------------------------------------
// Acceso volátil a registros (helpers privados).
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn reg_read(addr: usize) -> u32 {
    // SAFETY: `addr` es una dirección de registro MMIO válida del periférico.
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn reg_write(addr: usize, val: u32) {
    // SAFETY: idem; escritura de 32 bits alineada sobre MMIO.
    core::ptr::write_volatile(addr as *mut u32, val)
}

// ---------------------------------------------------------------------------
// Buffer TX en anillo (software).
// ---------------------------------------------------------------------------

/// Cola circular de bytes de tamaño fijo. Sin asignación dinámica.
struct Ring {
    buf: [u8; TX_BUF_LEN],
    head: usize, // índice de lectura (siguiente a extraer)
    tail: usize, // índice de escritura (siguiente hueco)
    len: usize,  // bytes ocupados
}

impl Ring {
    const fn new() -> Self {
        Self { buf: [0; TX_BUF_LEN], head: 0, tail: 0, len: 0 }
    }

    /// Encola un byte. Devuelve `false` si el anillo está lleno.
    fn push(&mut self, b: u8) -> bool {
        if self.len >= TX_BUF_LEN {
            return false;
        }
        // Índice acotado por construcción (tail < TX_BUF_LEN): acceso seguro.
        if let Some(slot) = self.buf.get_mut(self.tail) {
            *slot = b;
        }
        self.tail = (self.tail + 1) % TX_BUF_LEN;
        self.len += 1;
        true
    }

    /// Mira el siguiente byte sin extraerlo.
    fn peek(&self) -> Option<u8> {
        if self.len == 0 {
            None
        } else {
            self.buf.get(self.head).copied()
        }
    }

    /// Extrae el siguiente byte.
    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.buf.get(self.head).copied();
        self.head = (self.head + 1) % TX_BUF_LEN;
        self.len -= 1;
        b
    }
}

// ---------------------------------------------------------------------------
// Estado interno de la consola (protegido por SpinLock).
// ---------------------------------------------------------------------------

struct ConsoleInner {
    tx: Ring,
}

impl ConsoleInner {
    const fn new() -> Self {
        Self { tx: Ring::new() }
    }

    /// Vuelca el anillo TX al FIFO del endpoint IN y entrega el paquete.
    /// No bloquea indefinidamente: si el host no drena, abandona (los bytes
    /// que queden en el anillo se reintentan en la siguiente llamada a `write`).
    fn drain_tx(&mut self) {
        let mut wrote_any = false;
        let mut guard: u32 = 0;

        while let Some(b) = self.tx.peek() {
            let free = unsafe { reg_read(EP1_CONF_REG) } & CONF_IN_EP_DATA_FREE != 0;
            if !free {
                // Sin hueco: esperamos un poco. Si se agota el margen, salimos. [HW]
                guard = guard.wrapping_add(1);
                if guard > MAX_SPIN {
                    break;
                }
                core::hint::spin_loop();
                continue;
            }
            guard = 0;
            // Cargar el byte en el FIFO IN.
            unsafe { reg_write(EP1_REG, b as u32) };
            let _ = self.tx.pop();
            wrote_any = true;
        }

        if wrote_any {
            // WR_DONE: entrega ("flush") lo acumulado al host USB. [HW]
            unsafe { reg_write(EP1_CONF_REG, CONF_WR_DONE) };
        }
    }

    /// Lee un byte del FIFO RX si hay alguno disponible (no bloqueante).
    fn read_hw_byte(&mut self) -> Option<u8> {
        let avail = unsafe { reg_read(EP1_CONF_REG) } & CONF_OUT_EP_DATA_AVAIL != 0;
        if !avail {
            return None;
        }
        // El byte llega en los 8 bits bajos de EP1_REG.
        Some((unsafe { reg_read(EP1_REG) } & 0xFF) as u8)
    }
}

/// Envoltorio Sync sobre el estado de consola, protegido por un `SpinLock`.
///
/// Se usa `SpinLock` + `UnsafeCell` (nunca `static mut`) como exige el contrato.
/// NOTA: cuando exista la ISR de RX, las secciones que compartan este estado con
/// la interrupción deben tomarse con `interrupts::critical_section` para evitar
/// un interbloqueo del propio spinlock. [HW]
struct Console {
    lock: SpinLock,
    inner: UnsafeCell<ConsoleInner>,
}

// SAFETY: todo acceso a `inner` se serializa con `lock`.
unsafe impl Sync for Console {}

impl Console {
    const fn new() -> Self {
        Self { lock: SpinLock::new(), inner: UnsafeCell::new(ConsoleInner::new()) }
    }

    /// Ejecuta `f` con acceso exclusivo al estado interno.
    fn with<R>(&self, f: impl FnOnce(&mut ConsoleInner) -> R) -> R {
        self.lock.lock();
        // SAFETY: el lock garantiza exclusión mutua sobre `inner`.
        let r = f(unsafe { &mut *self.inner.get() });
        self.lock.unlock();
        r
    }
}

/// Estado global de la consola.
static CONSOLE: Console = Console::new();

/// Marca de inicialización (diagnóstico). El USB-Serial-JTAG lo deja utilizable
/// el ROM/segundo bootloader, así que `init` es principalmente ceremonial.
static INITIALIZED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// API pública (contrato §3.9).
// ---------------------------------------------------------------------------

/// Inicializa la consola (USB-Serial-JTAG). [CANÓNICO]
///
/// En este chip el periférico ya queda operativo tras el arranque, por lo que
/// aquí solo se deja constancia de la inicialización. [HW] Si se migrara a UART0
/// físico, este sería el lugar para fijar baudios, mapear TX/RX en el GPIO matrix
/// y habilitar el reloj del bloque.
pub fn init() -> KResult<()> {
    INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

/// Escribe `buf` en la consola. Devuelve cuántos bytes aceptó. [CANÓNICO]
///
/// Los bytes se encolan en el anillo TX y se vuelcan al FIFO hardware. Si el
/// anillo se llena (host lento/ausente) devuelve un recuento parcial en vez de
/// bloquear.
pub fn write(buf: &[u8]) -> usize {
    CONSOLE.with(|c| {
        let mut n = 0usize;
        for &b in buf {
            if !c.tx.push(b) {
                // Anillo lleno: intentamos vaciar y reintentar una vez.
                c.drain_tx();
                if !c.tx.push(b) {
                    break;
                }
            }
            n += 1;
        }
        c.drain_tx();
        n
    })
}

/// Lee bytes disponibles en `buf` (no bloqueante). Devuelve cuántos leyó. [CANÓNICO]
pub fn read(buf: &mut [u8]) -> usize {
    CONSOLE.with(|c| {
        let mut n = 0usize;
        while n < buf.len() {
            match c.read_hw_byte() {
                Some(b) => {
                    if let Some(slot) = buf.get_mut(n) {
                        *slot = b;
                    }
                    n += 1;
                }
                None => break,
            }
        }
        n
    })
}

/// Devuelve un byte de la consola, o `None` si no hay ninguno disponible. [CANÓNICO]
pub fn getc() -> Option<u8> {
    CONSOLE.with(|c| c.read_hw_byte())
}
