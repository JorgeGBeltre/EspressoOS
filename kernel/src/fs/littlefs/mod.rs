//! Integración (BORRADOR) de LittleFS sobre `drivers::flash`. — Fase 4.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! LittleFS se elige por su resistencia a cortes de energía (log-structured,
//! copy-on-write, wear-leveling). Este archivo enlaza la geometría del volumen
//! y las llamadas de bloque del núcleo `lfs` con nuestro `drivers::flash`, y lo
//! expone como `FileSystem` montable en el VFS (`/`).
//!
//! Estado: BORRADOR. La parte REAL implementada aquí es el "adaptador de
//! dispositivo de bloques": el mapeo (bloque, offset) -> offset absoluto de
//! flash y las llamadas `read`/`prog`/`erase`/`sync`, más la validación de la
//! región y el `format` (borrado de sectores). El NÚCLEO `lfs` (parseo de
//! metadatos, superbloques, directorios, ficheros) requiere una crate externa
//! (`littlefs2`, ver `needs_crates`) que aún NO está enlazada; mientras tanto la
//! raíz es un marcador y las operaciones de contenido devuelven `NotSupported`.
//!
//! ------------------------------------------------------------------------
//! Flujo `lfs_config` previsto (documentación del enlace pendiente):
//!
//!   struct lfs_config cfg = {
//!       .context      = &LfsConfig,           // nuestra config/adaptador
//!       .read         = dev_read,             // -> flash::read
//!       .prog         = dev_prog,             // -> flash::write
//!       .erase        = dev_erase,            // -> flash::erase_sector
//!       .sync         = dev_sync,             // no-op (flash síncrona)
//!       .read_size    = cfg.read_size,
//!       .prog_size    = cfg.prog_size,
//!       .block_size   = cfg.block_size,       // = SECTOR_SIZE (4096)
//!       .block_count  = cfg.block_count,      // = region_len / block_size
//!       .cache_size   = cfg.cache_size,
//!       .lookahead_size = cfg.lookahead_size,
//!       .block_cycles = cfg.block_cycles,     // wear-leveling
//!   };
//!   // Montaje:
//!   //   e = lfs_mount(&lfs, &cfg);
//!   //   if e == LFS_ERR_CORRUPT && format_if_empty { lfs_format(&lfs,&cfg); lfs_mount(...) }
//! ------------------------------------------------------------------------
#![allow(dead_code)]

use crate::drivers::flash;
use crate::prelude::*;
use crate::vfs::inode::{DirEntry, FileSystem, FsStat, Inode, InodeKind};

/// Parámetros de configuración del volumen LittleFS (equivalente a
/// `struct lfs_config`). Todos los tamaños derivan de la región de flash
/// asignada al FS (`layout::FS_OFFSET` / `layout::FS_SIZE`).
#[derive(Clone, Copy, Debug)]
pub struct LfsConfig {
    /// Offset absoluto en flash del bloque 0 del volumen.
    pub region_offset: u32,
    /// Tamaño total del volumen, en bytes.
    pub region_len: u32,
    /// Tamaño mínimo de lectura (bytes). En NOR flash puede ser pequeño; 16 es
    /// un valor conservador y alineado.
    pub read_size: u32,
    /// Tamaño mínimo de programación (bytes).
    pub prog_size: u32,
    /// Tamaño de bloque = sector de borrado (4096).
    pub block_size: u32,
    /// Número de bloques del volumen = `region_len / block_size`.
    pub block_count: u32,
    /// Ciclos de borrado por bloque antes de rotar (wear-leveling). LittleFS
    /// recomienda 100..1000; `-1` desactiva.
    pub block_cycles: i32,
    /// Tamaño de caché (múltiplo de `read_size`/`prog_size`).
    pub cache_size: u32,
    /// Tamaño del buffer *lookahead* del asignador de bloques (múltiplo de 8).
    pub lookahead_size: u32,
}

impl LfsConfig {
    /// Deriva la configuración estándar para la región `[offset, offset+len)`.
    pub const fn for_region(offset: u32, len: u32) -> LfsConfig {
        let block_size = flash::SECTOR_SIZE as u32; // 4096
        LfsConfig {
            region_offset: offset,
            region_len: len,
            read_size: 16,
            prog_size: 16,
            block_size,
            block_count: len / block_size, // block_size != 0 (const 4096)
            block_cycles: 500,
            cache_size: 256,
            lookahead_size: 32,
        }
    }

    /// Traduce (bloque, offset) del núcleo `lfs` a un offset absoluto de flash,
    /// comprobando límites del volumen y del bloque. Es la pieza central del
    /// adaptador de dispositivo de bloques.
    ///
    /// `abs = region_offset + block * block_size + off`, todo con aritmética
    /// comprobada para no desbordar en las rutas del kernel.
    fn abs_offset(&self, block: u32, off: u32, size: u32) -> KResult<u32> {
        if block >= self.block_count {
            return Err(KError::InvalidArgument);
        }
        // El acceso no puede cruzar el límite del bloque.
        match off.checked_add(size) {
            Some(fin) if fin <= self.block_size => {}
            _ => return Err(KError::InvalidArgument),
        }
        let block_byte = block
            .checked_mul(self.block_size)
            .ok_or(KError::InvalidArgument)?;
        let within = block_byte.checked_add(off).ok_or(KError::InvalidArgument)?;
        let abs = self
            .region_offset
            .checked_add(within)
            .ok_or(KError::InvalidArgument)?;
        Ok(abs)
    }

    /// Callback `read` de LittleFS: lee `buf` desde (bloque, off). REAL.
    fn dev_read(&self, block: u32, off: u32, buf: &mut [u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::read(abs, buf)
    }

    /// Callback `prog` de LittleFS: programa `buf` en (bloque, off). REAL.
    /// LittleFS garantiza que la región fue borrada antes de programar.
    fn dev_prog(&self, block: u32, off: u32, buf: &[u8]) -> KResult<()> {
        let abs = self.abs_offset(block, off, buf.len() as u32)?;
        flash::write(abs, buf)
    }

    /// Callback `erase` de LittleFS: borra un bloque completo. REAL.
    fn dev_erase(&self, block: u32) -> KResult<()> {
        let abs = self.abs_offset(block, 0, self.block_size)?;
        flash::erase_sector(abs)
    }

    /// Callback `sync` de LittleFS: la flash interna es síncrona -> no-op.
    fn dev_sync(&self) -> KResult<()> {
        Ok(())
    }
}

/// Raíz de marcador para el borrador de LittleFS. Se comporta como un
/// directorio vacío; toda operación de contenido devuelve `NotSupported` hasta
/// que se enlace el núcleo `lfs` (que envolverá `lfs_dir_*`/`lfs_file_*`).
struct LfsRoot;

impl Inode for LfsRoot {
    fn kind(&self) -> InodeKind {
        InodeKind::Dir
    }
    fn size(&self) -> u64 {
        0
    }
    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::IsADirectory)
    }
    fn write_at(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::IsADirectory)
    }
    fn truncate(&self, _len: u64) -> KResult<()> {
        Err(KError::IsADirectory)
    }
    fn readdir(&self, _index: usize) -> KResult<Option<DirEntry>> {
        // Directorio vacío mientras no haya núcleo lfs.
        Ok(None)
    }
    fn lookup(&self, _name: &str) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotFound)
    }
    fn create(&self, _name: &str, _kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotSupported)
    }
    fn unlink(&self, _name: &str) -> KResult<()> {
        Err(KError::NotSupported)
    }
}

/// Instancia (borrador) de un volumen LittleFS montado. Guarda la configuración
/// del adaptador y la raíz. Al enlazar `littlefs2`, `root` pasará a envolver el
/// directorio raíz real del volumen.
pub struct LittleFs {
    config: LfsConfig,
    root: Arc<LfsRoot>,
}

impl LittleFs {
    /// Monta el volumen LittleFS ubicado en `[offset, offset+len)` de la flash.
    /// Usar `layout::FS_OFFSET` / `layout::FS_SIZE`. Si no hay FS válido y
    /// `format_if_empty`, formatea (una vez enlazado el núcleo). [CANÓNICO]
    ///
    /// BORRADOR: valida la región y construye la configuración del adaptador.
    /// NO toca la flash automáticamente (no formatea) para no destruir datos de
    /// usuario mientras el núcleo `lfs` no esté enlazado. La raíz devuelta es un
    /// marcador; las operaciones de contenido devuelven `NotSupported`.
    pub fn mount(offset: u32, len: u32, format_if_empty: bool) -> KResult<Arc<LittleFs>> {
        Self::validar_region(offset, len)?;
        let config = LfsConfig::for_region(offset, len);

        // --- Enlace pendiente con el núcleo `lfs` (crate `littlefs2`) --------
        // 1. Construir `lfs_config` apuntando a dev_read/dev_prog/dev_erase/
        //    dev_sync (implementados arriba como métodos de `LfsConfig`).
        // 2. `let e = lfs_mount(&mut lfs, &cfg);`
        // 3. Si `e == LFS_ERR_CORRUPT` (volumen sin formatear) y
        //    `format_if_empty`: `lfs_format(&mut lfs, &cfg)` + reintento de mount.
        // 4. Envolver la raíz montada en un `Inode` que traduzca
        //    read_at/write_at/readdir/lookup/create/unlink a `lfs_*`.
        //
        // Hasta entonces NO formateamos aquí para no borrar la partición `fs`.
        let _ = format_if_empty;

        Ok(Arc::new(LittleFs {
            config,
            root: Arc::new(LfsRoot),
        }))
    }

    /// Formatea el volumen (DESTRUYE datos). [CANÓNICO]
    ///
    /// BORRADOR: realiza la parte real sobre `drivers::flash` — borra todos los
    /// sectores de la región. Un formateo LittleFS COMPLETO además escribe DOS
    /// superbloques (bloques 0 y 1) con los metadatos iniciales; eso lo hace
    /// `lfs_format` del núcleo (pendiente de enlazar). Tras este borrado, un
    /// `mount` con `format_if_empty` debería reconstruir los superbloques.
    pub fn format(offset: u32, len: u32) -> KResult<()> {
        Self::validar_region(offset, len)?;
        let sector = flash::SECTOR_SIZE as u32;
        let fin = offset.checked_add(len).ok_or(KError::InvalidArgument)?;
        let mut pos = offset;
        while pos < fin {
            flash::erase_sector(pos)?;
            pos = pos.checked_add(sector).ok_or(KError::InvalidArgument)?;
        }
        Ok(())
    }

    /// Validación de la región: no vacía, alineada a sector, múltiplo de sector
    /// y contenida en la flash.
    fn validar_region(offset: u32, len: u32) -> KResult<()> {
        let sector = flash::SECTOR_SIZE as u32;
        if len == 0 || sector == 0 {
            return Err(KError::InvalidArgument);
        }
        if offset % sector != 0 || len % sector != 0 {
            return Err(KError::InvalidArgument);
        }
        let fin = offset.checked_add(len).ok_or(KError::InvalidArgument)?;
        if fin > layout::FLASH_SIZE {
            return Err(KError::InvalidArgument);
        }
        Ok(())
    }
}

impl FileSystem for LittleFs {
    fn name(&self) -> &str {
        "littlefs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        let r: Arc<dyn Inode> = self.root.clone();
        r
    }

    fn sync(&self) -> KResult<()> {
        // Con `lfs` enlazado: vaciar cachés / `lfs_deinit`. Flash síncrona: no-op.
        Ok(())
    }

    fn stat(&self) -> FsStat {
        // `total` es conocido (tamaño del volumen). `used` real requiere
        // `lfs_fs_size()` del núcleo; en el borrador se reporta 0.
        FsStat {
            total_bytes: self.config.region_len as u64,
            used_bytes: 0,
            block_size: self.config.block_size,
        }
    }
}
