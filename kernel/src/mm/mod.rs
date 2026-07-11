//! Gestión de memoria del kernel: heap global y protección de regiones.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Fase 1: allocator global sobre SRAM interna + heap secundario en PSRAM
//! (ver [`heap`]). Fase 8: protección de regiones vía PMS/World Controller
//! (ver [`mpu`]).
//!
//! Este módulo re-exporta la API pública de diagnóstico del heap para que el
//! resto del kernel (p. ej. el comando `free` de la shell) la use como
//! `mm::{init, size, stats, HeapStats}` sin conocer la estructura interna.

pub mod heap;
pub mod mpu;

// Re-export canónico de la API del heap (§3.1 del contrato).
pub use heap::{init, size, stats, HeapStats};
