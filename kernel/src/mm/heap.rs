//! Allocator global del kernel: heap en SRAM interna + heap secundario en PSRAM.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Fase 0/1. El heap primario vive en SRAM interna (`layout::KERNEL_HEAP_SIZE`,
//! capacidad `MemoryCapability::Internal`). En Fase 1 se añade una segunda región
//! en PSRAM (hasta 8 MB, `MemoryCapability::External`) para buffers grandes de
//! FS/red mediante [`add_psram`].
//!
//! Toda la contabilidad de "capacidad total configurada" se lleva aquí en un
//! contador atómico (`TOTAL_CONFIGURED`), porque `esp-alloc` no expone de forma
//! estable la suma de todas las regiones registradas. El uso/libre en tiempo real
//! se consulta a `ALLOCATOR.used()` / `ALLOCATOR.free()` (§1.9 del contrato).
//!
//! NOTA de riesgo (esp-alloc 0.6.0): `HeapRegion::new` es `unsafe` (registra una
//! región de memoria cruda que DEBE estar disponible y no aliaseada). Los nombres
//! `used()`/`free()` están marcados `(?)` en el contrato; si el crate instalado
//! difiere, esta es la ÚNICA capa que hay que ajustar (el resto del kernel solo
//! ve `stats()`/`size()`).

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::prelude::layout;

/// Estadísticas del heap para diagnóstico (comando `free`). [CANÓNICO]
#[derive(Clone, Copy, Debug, Default)]
pub struct HeapStats {
    /// Capacidad total configurada (SRAM + PSRAM), en bytes.
    pub total: usize,
    /// Bytes actualmente en uso.
    pub used: usize,
    /// Bytes libres.
    pub free: usize,
}

/// Allocator global. `EspHeap::empty()` es `const`, así que puede vivir en un
/// `static` sin inicialización en tiempo de ejecución; las regiones se añaden en
/// [`init`] / [`add_psram`].
#[global_allocator]
static ALLOCATOR: esp_alloc::EspHeap = esp_alloc::EspHeap::empty();

/// Capacidad total configurada acumulada (SRAM + PSRAM), en bytes.
/// Fuente para [`size`] y para `HeapStats::total`.
static TOTAL_CONFIGURED: AtomicUsize = AtomicUsize::new(0);

/// Guardas de una-sola-vez: evitan registrar dos veces la misma región si algún
/// camino de arranque llama de más. No hay lock: son operaciones idempotentes.
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static PSRAM_ADDED: AtomicBool = AtomicBool::new(false);

/// Buffer estático del heap primario en SRAM interna. Alineado a 8 bytes para
/// satisfacer los requisitos de alineación del allocator sobre cualquier tipo.
///
/// Este `static mut` es la excepción documentada del contrato (§0.7): el buffer
/// del heap es el único estado global mutable permitido sin `Mutex`/`SpinLock`,
/// porque su propiedad se cede íntegra al allocator en [`init`].
#[repr(C, align(8))]
struct HeapBuf([u8; layout::KERNEL_HEAP_SIZE]);
static mut KERNEL_HEAP: HeapBuf = HeapBuf([0; layout::KERNEL_HEAP_SIZE]);

/// Inicializa el heap del kernel (región SRAM interna). Llamar UNA vez, temprano
/// en `main`, ANTES de cualquier asignación dinámica. [CANÓNICO]
///
/// Idempotente-seguro: una segunda llamada no vuelve a registrar la región.
pub fn init() {
    // Si ya se inicializó, no volver a añadir la región (evita doble registro).
    if INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }

    // Puntero al buffer estático. `addr_of_mut!` evita crear una referencia
    // intermedia a un `static mut` (comportamiento indefinido si se aliasea).
    let base = core::ptr::addr_of_mut!(KERNEL_HEAP) as *mut u8;
    let len = layout::KERNEL_HEAP_SIZE;

    // SAFETY: `KERNEL_HEAP` es un buffer estático exclusivo cuya propiedad se
    // cede aquí al allocator; `base`/`len` describen exactamente esa región,
    // válida durante toda la vida del programa y no usada por nadie más.
    unsafe {
        ALLOCATOR.add_region(esp_alloc::HeapRegion::new(
            base,
            len,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }

    // Contabilizar la capacidad configurada. `fetch_add` es ENVOLVENTE (no
    // saturante), pero `len` está acotado por `KERNEL_HEAP_SIZE` (64 KB) y el
    // total posible (SRAM + PSRAM) queda muy por debajo de `usize::MAX`, así que
    // no puede desbordar en la práctica (§0.5 del contrato).
    TOTAL_CONFIGURED.fetch_add(len, Ordering::Relaxed);
}

/// Añade la región PSRAM (externa) al allocator. Fase 1. [CANÓNICO]
///
/// `base`/`len` provienen del init de PSRAM (mapeo a caché específico de
/// `esp-hal`, §4 del contrato), que se realiza en `main` porque requiere el
/// periférico `PSRAM`; aquí solo se registra la región ya mapeada.
///
/// Idempotente-seguro: llamar una sola vez. Una `len` de 0 se ignora.
pub fn add_psram(base: *mut u8, len: usize) {
    // Nada que registrar.
    if len == 0 {
        return;
    }
    // Registrar la PSRAM como máximo una vez.
    if PSRAM_ADDED.swap(true, Ordering::AcqRel) {
        return;
    }

    // SAFETY: el llamador garantiza (contrato) que `[base, base+len)` es la
    // ventana de PSRAM ya inicializada y mapeada a caché, exclusiva del heap y
    // válida durante toda la vida del programa.
    unsafe {
        ALLOCATOR.add_region(esp_alloc::HeapRegion::new(
            base,
            len,
            esp_alloc::MemoryCapability::External.into(),
        ));
    }

    TOTAL_CONFIGURED.fetch_add(len, Ordering::Relaxed);
}

/// Capacidad total configurada, en bytes (SRAM + PSRAM registradas). [CANÓNICO]
///
/// Refleja las regiones añadidas hasta el momento: tras [`init`] son
/// `KERNEL_HEAP_SIZE`; tras [`add_psram`] incluye además la PSRAM.
pub fn size() -> usize {
    TOTAL_CONFIGURED.load(Ordering::Relaxed)
}

/// Instantánea de uso del heap. [CANÓNICO]
///
/// `used`/`free` se leen del allocator en vivo; `total` es la capacidad
/// configurada acumulada. Nunca panica ni hace overflow.
pub fn stats() -> HeapStats {
    HeapStats {
        total: TOTAL_CONFIGURED.load(Ordering::Relaxed),
        // `used()`/`free()`: superficie marcada `(?)` en el contrato (§1.9).
        // Si el crate instalado renombra estos métodos, ajustarlos AQUÍ.
        used: ALLOCATOR.used(),
        free: ALLOCATOR.free(),
    }
}
