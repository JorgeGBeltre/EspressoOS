//! Driver de la SPI flash interna (Fase 4).
//!
//! Da acceso de lectura/escritura/borrado a la NOR flash SPI de 16 MB del
//! ESP32-S3. Es el backend de bloques del sistema de archivos (LittleFS, `/`)
//! y del subsistema OTA (slots A/B + `otadata`).
//!
//! ## Modelo de la NOR flash (IMPORTANTE)
//! Una NOR flash sólo puede pasar bits de `1` a `0` al escribir. Para volver a
//! poner bits a `1` hay que BORRAR, y el borrado sólo funciona por SECTORES
//! completos de [`SECTOR_SIZE`] (4 KB). Reglas que respeta este driver y que el
//! llamador debe conocer:
//!
//! 1. **Borrar antes de escribir.** [`write`] NO borra: asume que la región de
//!    destino ya está en estado borrado (`0xFF`). Escribir sobre datos sin
//!    borrar produce una mezcla AND indefinida, no lo que se quería. El orden
//!    correcto es `erase_sector` → `write`.
//! 2. **Alineación de escritura.** El controlador ROM escribe por PALABRAS de 4
//!    bytes: tanto el `offset` como la longitud del buffer deben ser múltiplos
//!    de 4. En caso contrario se devuelve [`KError::InvalidArgument`] (no se
//!    intenta un read-modify-write silencioso, que sería incorrecto en NOR).
//! 3. **Granularidad de borrado.** [`erase_sector`] borra el sector de 4 KB que
//!    contiene el `offset` dado (redondea hacia abajo al inicio del sector).
//!    Borra 4 KB completos: cualquier dato vecino dentro de ese sector se
//!    pierde. El planificador de FS/OTA debe alinear sus estructuras a 4 KB.
//! 4. **Rango.** Todo acceso se valida contra `layout::FLASH_SIZE`. Un acceso
//!    que se saldría del chip devuelve [`KError::InvalidArgument`].
//!
//! ## Backend
//! Se envuelve la crate `esp-storage` (`FlashStorage`), que implementa los
//! traits `embedded-storage`/`NorFlash` llamando a las rutinas ROM SPI de
//! Espressif. `esp-storage` gestiona internamente la sección crítica y el
//! desalojo de caché necesarios durante cada operación de flash, así que aquí
//! sólo se serializa el acceso con un `Mutex` del kernel.
//!
//! > NOTA (incierto): la versión exacta de `esp-storage` compatible con
//! > esp-hal 0.23 y la superficie de sus traits son el punto más frágil de este
//! > archivo. Si difiere del instalado, ESTE módulo es la única capa que debe
//! > absorberlo: el resto del kernel sólo ve `read/write/erase_sector`.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

// Traits que aportan los métodos `read`, `write` y `erase` sobre `FlashStorage`.
// Se importan SÓLO los de `nor_flash` para evitar la ambigüedad con el `write`
// del trait `embedded_storage::Storage` (que hace read-modify-write y NO es lo
// que queremos en una NOR flash cruda).
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::FlashStorage;

/// Tamaño de sector de borrado (bytes). Debe coincidir con
/// `layout::FLASH_SECTOR_SIZE` y con el `ERASE_SIZE` del backend NOR.
pub const SECTOR_SIZE: usize = 4096;

/// Alineación mínima de escritura de la NOR flash de la S3 (palabra de 4 bytes).
/// El `offset` y la longitud de todo [`write`] deben ser múltiplos de esto.
const WRITE_ALIGN: u32 = 4;

/// Backend global de la flash, protegido y con inicialización perezosa.
///
/// `esp-storage` construye `FlashStorage` leyendo parámetros del chip vía ROM,
/// por eso no puede ser `const`; se crea en el primer acceso. Se guarda tras un
/// `Mutex` porque todos sus métodos toman `&mut self` y varias tareas (FS y OTA)
/// comparten la flash: el lock serializa los accesos.
static FLASH: Mutex<Option<FlashStorage>> = Mutex::new(None);

/// Toma el lock del backend, lo crea si aún no existe, y ejecuta `f` con acceso
/// exclusivo. Nunca entra en pánico: `get_or_insert_with` garantiza `&mut` sin
/// desempaquetar un `Option` que pudiera ser `None`.
fn with_flash<R>(f: impl FnOnce(&mut FlashStorage) -> R) -> R {
    let mut guard = FLASH.lock();
    let fs = guard.get_or_insert_with(FlashStorage::new);
    f(fs)
}

/// Valida que `[offset, offset + len)` cae dentro del chip de flash.
/// Usa aritmética de 64 bits para que la suma no pueda desbordar. Devuelve
/// [`KError::InvalidArgument`] si el rango se sale o `offset+len` desborda.
fn check_range(offset: u32, len: usize) -> KResult<()> {
    let end = (offset as u64)
        .checked_add(len as u64)
        .ok_or(KError::InvalidArgument)?;
    if end > layout::FLASH_SIZE as u64 {
        return Err(KError::InvalidArgument);
    }
    Ok(())
}

/// Dirección de inicio del sector de 4 KB que contiene `offset`
/// (redondeo hacia abajo). `SECTOR_SIZE` es potencia de 2, así que basta la
/// máscara; no hay resta que pueda desbordar.
const fn sector_base(offset: u32) -> u32 {
    offset & !((SECTOR_SIZE as u32) - 1)
}

/// Lee `buf.len()` bytes desde `offset` de la flash. [CANÓNICO]
///
/// La lectura NO requiere borrado previo. Sobre `esp-storage` la lectura de
/// granularidad byte puede requerir la feature `bytewise-read` según versión
/// (ver `needs_crates`); sin ella el backend podría exigir `offset`/longitud
/// alineados a 4 bytes. El FS debe configurar su `read_size` en consecuencia.
pub fn read(offset: u32, buf: &mut [u8]) -> KResult<()> {
    check_range(offset, buf.len())?;
    // `read` proviene de `ReadNorFlash`. Traducimos el error del backend a
    // `KError::IoError` en la frontera del driver (nunca propagamos su tipo).
    with_flash(|fs| fs.read(offset, buf).map_err(|_| KError::IoError))
}

/// Escribe `buf` a partir de `offset`. La región DEBE estar borrada. [CANÓNICO]
///
/// Precondiciones (si no se cumplen -> [`KError::InvalidArgument`]):
/// - `offset` múltiplo de 4.
/// - `buf.len()` múltiplo de 4.
/// - `[offset, offset+len)` dentro del chip.
///
/// El sector destino debe haberse borrado antes con [`erase_sector`]; escribir
/// sobre bytes no borrados sólo puede apagar bits, corrompiendo el dato.
pub fn write(offset: u32, buf: &[u8]) -> KResult<()> {
    check_range(offset, buf.len())?;
    // La NOR flash escribe por palabras: exigir alineación de offset y longitud.
    if offset % WRITE_ALIGN != 0 || (buf.len() as u32) % WRITE_ALIGN != 0 {
        return Err(KError::InvalidArgument);
    }
    if buf.is_empty() {
        return Ok(()); // escribir 0 bytes es un no-op válido.
    }
    // `write` proviene de `NorFlash` (escritura cruda, sin auto-borrado).
    with_flash(|fs| fs.write(offset, buf).map_err(|_| KError::IoError))
}

/// Borra el sector de 4 KB que contiene `offset` (lo redondea al inicio de
/// sector) dejándolo todo a `0xFF`. [CANÓNICO]
///
/// Borra 4 KB completos: todo dato dentro de ese sector se pierde. Un `offset`
/// fuera del chip devuelve [`KError::InvalidArgument`].
pub fn erase_sector(offset: u32) -> KResult<()> {
    let base = sector_base(offset);
    // Validar el sector completo; `base + SECTOR_SIZE` no puede desbordar porque
    // `FLASH_SIZE` es múltiplo de `SECTOR_SIZE` y `base < FLASH_SIZE`.
    check_range(base, SECTOR_SIZE)?;
    let end = base + SECTOR_SIZE as u32;
    // `erase(from, to)` de `NorFlash`: `to` es exclusivo y ambos extremos deben
    // estar alineados a `ERASE_SIZE` (garantizado: `base` alineado, `end` = +4K).
    with_flash(|fs| fs.erase(base, end).map_err(|_| KError::IoError))
}
