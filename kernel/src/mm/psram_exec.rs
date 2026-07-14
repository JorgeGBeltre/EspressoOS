#![allow(dead_code)]

//! Ejecución de código desde PSRAM (Ruta B del userland).
//!
//! El ESP32-S3 es Harvard: `0x3c000000+` (PSRAM/flash por el bus de DATOS) NO es
//! fetchable. Para ejecutar código de usuario desde la PSRAM reservada hay que
//! mapear sus páginas físicas también en el **bus de INSTRUCCIONES** (`0x42xxxxxx`)
//! vía la MMU. esp-hal ya mapeó la PSRAM en el bus de datos (`psram_base`); aquí
//! añadimos el mapeo de instrucciones de las MISMAS páginas físicas → quedan
//! dual-mapeadas: se **escribe** el código por el alias de datos y se **ejecuta**
//! por el alias de instrucciones.
//!
//! Símbolos ROM (confirmados en `esp32s3.rom.ld`): `Cache_Ibus_MMU_Set`,
//! `Cache_WriteBack_All`, `Cache_Invalidate_ICache_All`.

/// `DPORT_MMU_ACCESS_SPIRAM` — la entrada MMU apunta a PSRAM (no flash).
const MMU_ACCESS_SPIRAM: u32 = 1 << 15;

/// Tamaño de página de la MMU externa (64 KB).
pub const MMU_PAGE_SIZE: u32 = 0x1_0000;

/// Base del bus de instrucciones donde exponemos la PSRAM de userland. Elegida
/// bien por encima de la IROM de flash del kernel (~`0x4209_xxxx`).
pub const USER_IBUS_BASE: u32 = 0x4280_0000;

/// Tamaño de la región de userland (1 MB reservado en PSRAM).
pub const USER_REGION_SIZE: u32 = 0x10_0000;

use core::sync::atomic::{AtomicU32, Ordering};

/// Alias de DATOS de la página física 0 de la región de userland (= `psram_base`,
/// p.ej. 0x3c0e0000). Lo fija `main` tras mapear la PSRAM. El loader escribe el
/// `.text` (cuyo vaddr está en el bus de instrucciones) a su alias de datos.
static DATA_BASE: AtomicU32 = AtomicU32::new(0);

pub fn set_data_base(base: u32) {
    DATA_BASE.store(base, Ordering::Relaxed);
}

pub fn user_data_base() -> u32 {
    DATA_BASE.load(Ordering::Relaxed)
}

/// Alias de DATOS de una dirección del bus de instrucciones de userland (para
/// poder ESCRIBIR ahí el código que luego se EJECUTA por el bus de instrucciones).
pub fn ibus_to_data(instr_addr: u32) -> u32 {
    user_data_base() + (instr_addr - USER_IBUS_BASE)
}

pub fn is_ibus(addr: u32, size: u32) -> bool {
    addr >= USER_IBUS_BASE && addr.saturating_add(size) <= USER_IBUS_BASE + USER_REGION_SIZE
}

extern "C" {
    /// Fija el mapeo MMU del ICache. Misma firma que `cache_dbus_mmu_set`:
    /// (ext_ram, vaddr, paddr, psize_kb, num_pages, fixed) -> 0 si OK.
    fn Cache_Ibus_MMU_Set(
        ext_ram: u32,
        vaddr: u32,
        paddr: u32,
        psize: u32,
        num: u32,
        fixed: u32,
    ) -> i32;

    fn Cache_WriteBack_All();
    fn Cache_Invalidate_ICache_All();
}

/// Mapea `num_pages` páginas físicas de PSRAM (desde `phys_page0`) al bus de
/// instrucciones en [`USER_IBUS_BASE`]. Devuelve `Err(code)` si el ROM falla.
pub fn map_instruction(phys_page0: u32, num_pages: u32) -> Result<(), i32> {
    let r = unsafe {
        Cache_Ibus_MMU_Set(
            MMU_ACCESS_SPIRAM,
            USER_IBUS_BASE,
            phys_page0 * MMU_PAGE_SIZE, // paddr alineado a página
            64,                         // psize en KB
            num_pages,
            0, // 0 = páginas físicas crecen con las virtuales
        )
    };
    if r != 0 {
        Err(r)
    } else {
        Ok(())
    }
}

/// Dirección del bus de INSTRUCCIONES para un offset dentro de la región de
/// userland (0..1 MB).
#[inline]
pub fn ibus_addr(offset: u32) -> u32 {
    USER_IBUS_BASE + offset
}

/// Tras escribir código por el bus de datos, sincroniza cachés para que el fetch
/// por el bus de instrucciones vea los bytes recién escritos: primero vuelca la
/// DCache a la PSRAM, luego invalida la ICache.
pub fn sync_caches() {
    unsafe {
        Cache_WriteBack_All();
        Cache_Invalidate_ICache_All();
    }
}

// Función-sonda call0 (sin ventana, sin literales): `movi a2, 42; ret`. Es
// position-independent (no referencias absolutas), así que sus bytes se pueden
// copiar a cualquier sitio y ejecutar. Definida con global_asm para no necesitar
// la feature `naked_functions`.
core::arch::global_asm!(
    ".section .rwtext,\"ax\",@progbits",
    ".align 4",
    ".global __psram_probe_tpl",
    ".global __psram_probe_end",
    "__psram_probe_tpl:",
    "movi a2, 42",
    "ret",
    "__psram_probe_end:",
);

extern "C" {
    fn __psram_probe_tpl();
    fn __psram_probe_end();
}

/// Autotest de Paso 1: copia la sonda al alias de DATOS de la página física 0 de
/// la región de userland (`data_base`), sincroniza cachés y la ejecuta por el bus
/// de INSTRUCCIONES. Debe devolver 42. Si la MMU/caché están mal, esto provoca un
/// `InstrFetchError` (lo cual también nos dice exactamente qué falló).
pub fn selftest(data_base: u32) -> u32 {
    let src = __psram_probe_tpl as *const () as usize;
    let end = __psram_probe_end as *const () as usize;
    let len = end.saturating_sub(src).max(8);

    unsafe {
        core::ptr::copy_nonoverlapping(src as *const u8, data_base as *mut u8, len);
    }
    sync_caches();

    let ret: u32;
    unsafe {
        // callx0 machaca a0 (dir. de retorno); LLVM no permite a0 como operando,
        // así que lo guardamos/restauramos dentro del bloque (net preservado).
        core::arch::asm!(
            "mov {t}, a0",
            "callx0 {addr}",
            "mov a0, {t}",
            addr = in(reg) USER_IBUS_BASE,
            t = out(reg) _,
            lateout("a2") ret,
        );
    }
    ret
}
