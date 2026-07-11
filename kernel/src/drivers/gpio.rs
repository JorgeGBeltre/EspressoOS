//! Driver GPIO del ESP32-S3: configuración, escritura, lectura y toggle de pines.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Control directo por registros del periférico GPIO (0x6000_4000). Se cubren
//! los pines 0..=48 del S3 mediante los dos bancos de registros (0..31 y 32..48).
//!
//! Lo que SÍ es lógica real y completa:
//!   * Habilitar/deshabilitar salida (registros ENABLE / ENABLE1, con W1TS/W1TC).
//!   * Fijar nivel de salida atómicamente (OUT/OUT1 W1TS/W1TC).
//!   * Leer el nivel de un pin (IN / IN1).
//!   * Conmutar (`toggle`) leyendo el latch de salida.
//!   * Contabilidad de qué pines están en modo salida (para rechazar escrituras
//!     sobre pines de entrada con `PermissionDenied`).
//!
//! # Notas de validación en hardware [HW]
//!   * `configure(Output)` enruta el pad a "salida GPIO simple" escribiendo el
//!     índice de señal 128 en `GPIO_FUNCn_OUT_SEL_CFG`. NO toca IO_MUX
//!     (MCU_SEL/drive strength/FUN_IE): el ROM ya deja la mayoría de pines en
//!     función GPIO, pero conviene confirmarlo en la placa concreta.
//!   * Para lecturas de entrada fiables suele hacer falta habilitar `FUN_IE` en
//!     el registro IO_MUX del pad; se omite aquí a propósito (marcado [HW]).
//!   * Pines reservados a la flash/PSRAM (típicamente 26..=32 según encapsulado)
//!     NO deben tocarse; este driver no los bloquea, es responsabilidad del
//!     llamante. Validar contra el esquemático.
#![allow(dead_code)]

use crate::prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

/// Dirección (modo) de un pin GPIO. [CANÓNICO]
pub enum PinMode {
    Input,
    Output,
}

// ---------------------------------------------------------------------------
// Mapa de registros del periférico GPIO (ESP32-S3). [HW]
// ---------------------------------------------------------------------------

const GPIO_BASE: usize = 0x6000_4000;

// Banco bajo: pines 0..=31.
const GPIO_OUT_REG: usize = GPIO_BASE + 0x0004; // latch de salida (lectura para toggle)
const GPIO_OUT_W1TS: usize = GPIO_BASE + 0x0008; // poner a 1
const GPIO_OUT_W1TC: usize = GPIO_BASE + 0x000C; // poner a 0
const GPIO_ENABLE_W1TS: usize = GPIO_BASE + 0x0024; // habilitar salida
const GPIO_ENABLE_W1TC: usize = GPIO_BASE + 0x0028; // deshabilitar salida
const GPIO_IN_REG: usize = GPIO_BASE + 0x003C; // nivel de entrada

// Banco alto: pines 32..=48 (bit = pin - 32).
const GPIO_OUT1_REG: usize = GPIO_BASE + 0x0010;
const GPIO_OUT1_W1TS: usize = GPIO_BASE + 0x0014;
const GPIO_OUT1_W1TC: usize = GPIO_BASE + 0x0018;
const GPIO_ENABLE1_W1TS: usize = GPIO_BASE + 0x0030;
const GPIO_ENABLE1_W1TC: usize = GPIO_BASE + 0x0034;
const GPIO_IN1_REG: usize = GPIO_BASE + 0x0040;

/// Base de los registros `GPIO_FUNCn_OUT_SEL_CFG` (uno por pin, paso 4 bytes).
const GPIO_FUNC_OUT_SEL_BASE: usize = GPIO_BASE + 0x0554;
/// Índice de señal "salida GPIO por software" en la matriz de E/S. [HW]
const SIG_GPIO_OUT_IDX: u32 = 128;

/// Número de GPIO más alto del ESP32-S3.
const MAX_GPIO: u8 = 48;

// ---------------------------------------------------------------------------
// Contabilidad de pines en modo salida (sin bloqueo, seguro para ISR).
// Bit i del banco bajo -> pin i (0..31). Bit j del banco alto -> pin 32+j.
// ---------------------------------------------------------------------------

static OUTPUT_MASK_LO: AtomicU32 = AtomicU32::new(0);
static OUTPUT_MASK_HI: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Acceso volátil a registros (helpers privados).
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn reg_read(addr: usize) -> u32 {
    // SAFETY: `addr` es un registro MMIO válido del periférico GPIO.
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn reg_write(addr: usize, val: u32) {
    // SAFETY: idem; escritura de 32 bits alineada sobre MMIO.
    core::ptr::write_volatile(addr as *mut u32, val)
}

/// Valida que el número de pin exista en el S3.
fn check_pin(pin: u8) -> KResult<()> {
    if pin > MAX_GPIO {
        Err(KError::InvalidArgument)
    } else {
        Ok(())
    }
}

/// `true` si el pin está registrado como salida.
fn is_output(pin: u8) -> bool {
    if pin < 32 {
        OUTPUT_MASK_LO.load(Ordering::Relaxed) & (1u32 << pin) != 0
    } else {
        OUTPUT_MASK_HI.load(Ordering::Relaxed) & (1u32 << (pin - 32)) != 0
    }
}

/// Marca/desmarca un pin como salida en la contabilidad interna.
fn mark_output(pin: u8, is_out: bool) {
    if pin < 32 {
        let bit = 1u32 << pin;
        if is_out {
            OUTPUT_MASK_LO.fetch_or(bit, Ordering::Relaxed);
        } else {
            OUTPUT_MASK_LO.fetch_and(!bit, Ordering::Relaxed);
        }
    } else {
        let bit = 1u32 << (pin - 32);
        if is_out {
            OUTPUT_MASK_HI.fetch_or(bit, Ordering::Relaxed);
        } else {
            OUTPUT_MASK_HI.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

/// Habilita (o deshabilita) el driver de salida del pin a nivel de hardware.
fn set_enable(pin: u8, enable: bool) {
    if pin < 32 {
        let reg = if enable { GPIO_ENABLE_W1TS } else { GPIO_ENABLE_W1TC };
        unsafe { reg_write(reg, 1u32 << pin) };
    } else {
        let reg = if enable { GPIO_ENABLE1_W1TS } else { GPIO_ENABLE1_W1TC };
        unsafe { reg_write(reg, 1u32 << (pin - 32)) };
    }
}

/// Fija el nivel de salida de un pin de forma atómica (W1TS/W1TC).
fn set_level(pin: u8, high: bool) {
    if pin < 32 {
        let reg = if high { GPIO_OUT_W1TS } else { GPIO_OUT_W1TC };
        unsafe { reg_write(reg, 1u32 << pin) };
    } else {
        let reg = if high { GPIO_OUT1_W1TS } else { GPIO_OUT1_W1TC };
        unsafe { reg_write(reg, 1u32 << (pin - 32)) };
    }
}

/// Lee el latch de SALIDA del pin (lo que se está conduciendo), para `toggle`.
fn read_output_latch(pin: u8) -> bool {
    if pin < 32 {
        unsafe { reg_read(GPIO_OUT_REG) } & (1u32 << pin) != 0
    } else {
        unsafe { reg_read(GPIO_OUT1_REG) } & (1u32 << (pin - 32)) != 0
    }
}

// ---------------------------------------------------------------------------
// API pública (contrato §3.9).
// ---------------------------------------------------------------------------

/// Configura un pin como entrada o salida. [CANÓNICO]
pub fn configure(pin: u8, mode: PinMode) -> KResult<()> {
    check_pin(pin)?;
    match mode {
        PinMode::Output => {
            // Enrutar el pad a "salida GPIO simple" vía matriz de E/S (mejor
            // esfuerzo; ver [HW] en la cabecera). Paso 4 bytes por pin.
            unsafe {
                reg_write(GPIO_FUNC_OUT_SEL_BASE + (pin as usize) * 4, SIG_GPIO_OUT_IDX)
            };
            // Habilitar el driver de salida.
            set_enable(pin, true);
            mark_output(pin, true);
        }
        PinMode::Input => {
            // Deshabilitar salida; el pad queda de alta impedancia (entrada).
            // [HW] Para lecturas fiables puede requerirse habilitar FUN_IE en
            // IO_MUX del pad; se deja pendiente de validación en placa.
            set_enable(pin, false);
            mark_output(pin, false);
        }
    }
    Ok(())
}

/// Escribe un nivel lógico en un pin configurado como salida. [CANÓNICO]
///
/// Devuelve `PermissionDenied` si el pin no está en modo salida.
pub fn write(pin: u8, high: bool) -> KResult<()> {
    check_pin(pin)?;
    if !is_output(pin) {
        return Err(KError::PermissionDenied);
    }
    set_level(pin, high);
    Ok(())
}

/// Lee el nivel lógico de un pin. [CANÓNICO]
///
/// Válido tanto en entrada como en salida (en salida devuelve el nivel real
/// muestreado en el pad, no solo el latch).
pub fn read(pin: u8) -> KResult<bool> {
    check_pin(pin)?;
    let level = if pin < 32 {
        unsafe { reg_read(GPIO_IN_REG) } & (1u32 << pin) != 0
    } else {
        unsafe { reg_read(GPIO_IN1_REG) } & (1u32 << (pin - 32)) != 0
    };
    Ok(level)
}

/// Conmuta el nivel de un pin de salida. [CANÓNICO]
///
/// Devuelve `PermissionDenied` si el pin no está en modo salida.
pub fn toggle(pin: u8) -> KResult<()> {
    check_pin(pin)?;
    if !is_output(pin) {
        return Err(KError::PermissionDenied);
    }
    let current = read_output_latch(pin);
    set_level(pin, !current);
    Ok(())
}
