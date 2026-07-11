//! Inodos y sistemas de archivos del VFS: traits `Inode` y `FileSystem`.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Modelo "todo-es-un-archivo": cada objeto del árbol (fichero, directorio o
//! dispositivo) se maneja como `Arc<dyn Inode>`. Un `FileSystem` montable
//! entrega su raíz (un directorio) y se sincroniza/consulta como un todo.
//!
//! Reglas del contrato (§3.4.1):
//!  - `VfsError` es un ALIAS de [`KError`], para no fragmentar los errores.
//!  - `Inode` y `FileSystem` son `Send + Sync` (previsión SMP, Fase 9).
//!  - Ninguna ruta panica; se devuelve siempre `KResult`.
#![allow(dead_code)]

use crate::prelude::*;

/// Error del VFS. ALIAS de [`KError`] para no fragmentar el universo de errores.
pub type VfsError = KError;

/// Tipo de nodo del sistema de archivos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InodeKind {
    /// Fichero regular (contenido de bytes, soporta `read_at`/`write_at`).
    File,
    /// Directorio (soporta `readdir`/`lookup`/`create`/`unlink`).
    Dir,
    /// Dispositivo de carácter/bloque expuesto en `/dev`.
    Device,
    /// Enlace simbólico (reservado; resolución en fase posterior).
    Symlink,
}

/// Entrada de directorio devuelta por [`Inode::readdir`].
#[derive(Clone, Debug)]
pub struct DirEntry {
    /// Nombre corto de la entrada (sin la ruta del directorio).
    pub name: String,
    /// Tipo del nodo referenciado.
    pub kind: InodeKind,
    /// Número de inodo (identificador estable dentro del FS).
    pub ino: u64,
}

/// Un nodo del árbol de archivos: fichero, directorio o dispositivo.
///
/// Se maneja siempre como `Arc<dyn Inode>`. Debe ser `Send + Sync`.
///
/// Los métodos específicos de directorio (`readdir`, `lookup`, `create`,
/// `unlink`) y `truncate` tienen implementación por defecto que devuelve un
/// error semántico (nunca panican). Un FS concreto sólo sobreescribe los que
/// soporta. `read_at`/`write_at` son obligatorios: un directorio debe
/// implementarlos devolviendo [`KError::IsADirectory`].
pub trait Inode: Send + Sync {
    /// Tipo de este nodo.
    fn kind(&self) -> InodeKind;

    /// Tamaño en bytes (0 para directorios/dispositivos sin longitud).
    fn size(&self) -> u64;

    /// Lee hasta `buf.len()` bytes desde el offset `off`. Devuelve leídos.
    fn read_at(&self, off: u64, buf: &mut [u8]) -> KResult<usize>;

    /// Escribe `buf` a partir de `off`. Devuelve bytes escritos.
    fn write_at(&self, off: u64, buf: &[u8]) -> KResult<usize>;

    /// Ajusta el tamaño (sólo `File`). Por defecto: no soportado.
    fn truncate(&self, _len: u64) -> KResult<()> {
        Err(KError::NotSupported)
    }

    /// Itera las entradas de un directorio por índice. `Ok(None)` = fin.
    /// Por defecto: el nodo no es un directorio.
    fn readdir(&self, _index: usize) -> KResult<Option<DirEntry>> {
        Err(KError::NotADirectory)
    }

    /// Resuelve un hijo por nombre (sólo `Dir`). Por defecto: no es directorio.
    fn lookup(&self, _name: &str) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotADirectory)
    }

    /// Crea un hijo (sólo `Dir`). Por defecto: no es directorio.
    fn create(&self, _name: &str, _kind: InodeKind) -> KResult<Arc<dyn Inode>> {
        Err(KError::NotADirectory)
    }

    /// Elimina un hijo por nombre (sólo `Dir`). Por defecto: no es directorio.
    fn unlink(&self, _name: &str) -> KResult<()> {
        Err(KError::NotADirectory)
    }

    /// Fuerza la persistencia de este nodo (no-op en FS volátiles).
    fn sync(&self) -> KResult<()> {
        Ok(())
    }
}

/// Estadísticas de un FS montado (para `df`/diagnóstico).
#[derive(Clone, Copy, Debug, Default)]
pub struct FsStat {
    /// Capacidad total en bytes.
    pub total_bytes: u64,
    /// Bytes en uso.
    pub used_bytes: u64,
    /// Tamaño de bloque en bytes.
    pub block_size: u32,
}

/// Un sistema de archivos montable en el árbol del VFS.
pub trait FileSystem: Send + Sync {
    /// Nombre del FS ("ramfs", "littlefs", "devfs").
    fn name(&self) -> &str;

    /// Raíz del FS (siempre un directorio).
    fn root(&self) -> Arc<dyn Inode>;

    /// Sincroniza todo el FS a almacenamiento (no-op en FS volátiles).
    fn sync(&self) -> KResult<()>;

    /// Uso de espacio del FS.
    fn stat(&self) -> FsStat;
}
