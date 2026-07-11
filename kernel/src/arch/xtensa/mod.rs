//! Soporte de arquitectura Xtensa LX7 (ESP32-S3).
//!
//! Agrupa las cuatro capas dependientes de arquitectura y reexporta sus tipos
//! y funciones más usados para poder escribir rutas cortas del estilo
//! `arch::xtensa::Mutex` o `arch::xtensa::Context` desde el resto del kernel.
//!
//! Submódulos:
//! - [`context`]: `Context` + cambio de contexto (`switch_to`, `init_task_stack`).
//! - [`interrupts`]: enmascarado global (`disable`/`restore`/`critical_section`).
//! - [`sync`]: primitivas de sincronización (`SpinLock`, `Mutex`, `CriticalSection`).
//! - [`timer`]: systick del scheduler y reloj monotónico (`uptime_ms`, `TICK_HZ`).
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

pub mod context;
pub mod interrupts;
pub mod sync;
pub mod timer;

// -----------------------------------------------------------------------------
// Reexports de conveniencia (rutas cortas).
//
// NOTA: las funciones `init` NO se reexportan a propósito, porque tanto
// `interrupts::init` como `timer::init` (y otras) colisionarían en el mismo
// espacio de nombres. Para inicializar, usar la ruta cualificada:
//     arch::xtensa::interrupts::init();
//     arch::xtensa::timer::init();
// -----------------------------------------------------------------------------

// Cambio de contexto y estado de CPU (Fase 2).
pub use context::{switch_to, Context};

// Enmascarado global de interrupciones (Fase 3).
pub use interrupts::{disable, restore};

// Sincronización: cerrojos y sección crítica con guard.
pub use sync::{CriticalSection, Mutex, MutexGuard, SpinLock};

// Reloj / tick del scheduler.
pub use timer::{uptime_ms, TICK_HZ};
