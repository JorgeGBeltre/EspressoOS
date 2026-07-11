//! Descriptores de archivo abiertos: `Fd`, `OpenFlags`, `SeekFrom`, `OpenFile`.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Un [`OpenFile`] combina un inodo (`Arc<dyn Inode>`), la posición actual y el
//! modo de acceso. La tabla global de descriptores vive en [`crate::vfs`]
//! (mod.rs); aquí sólo se define el objeto de archivo abierto y su semántica de
//! lectura/escritura/posicionamiento. Nunca panica: aritmética comprobada.
#![allow(dead_code)]

use crate::prelude::*;
use super::inode::Inode;

/// Descriptor de archivo (índice en la tabla del proceso/kernel).
pub type Fd = i32;

/// Flags de apertura estilo POSIX, sin dependencia externa.
///
/// Los tres primeros bits codifican el modo de acceso:
/// `RDONLY` (bit 0), `WRONLY` (bit 1), `RDWR` = ambos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpenFlags(pub u32);

impl OpenFlags {
    /// Sólo lectura.
    pub const RDONLY: OpenFlags = OpenFlags(0x0001);
    /// Sólo escritura.
    pub const WRONLY: OpenFlags = OpenFlags(0x0002);
    /// Lectura y escritura.
    pub const RDWR: OpenFlags = OpenFlags(0x0003);
    /// Crear si no existe.
    pub const CREATE: OpenFlags = OpenFlags(0x0100);
    /// Añadir al final en cada escritura.
    pub const APPEND: OpenFlags = OpenFlags(0x0200);
    /// Truncar a 0 al abrir (si es escribible).
    pub const TRUNC: OpenFlags = OpenFlags(0x0400);

    /// `true` si TODOS los bits de `f` están presentes en `self`.
    pub const fn contains(self, f: OpenFlags) -> bool {
        (self.0 & f.0) == f.0
    }
}

/// Origen para [`OpenFile::seek`].
pub enum SeekFrom {
    /// Desde el inicio del archivo.
    Start(u64),
    /// Relativo a la posición actual (puede ser negativo).
    Current(i64),
    /// Relativo al final del archivo (puede ser negativo).
    End(i64),
}

/// Archivo abierto: inodo + posición + modo de acceso.
pub struct OpenFile {
    /// Inodo subyacente compartido.
    pub inode: Arc<dyn Inode>,
    /// Posición actual (offset de lectura/escritura).
    pub offset: u64,
    /// Se puede leer.
    pub readable: bool,
    /// Se puede escribir.
    pub writable: bool,
    /// Modo "append": cada escritura va al final.
    pub append: bool,
}

/// Suma con signo comprobada de un offset (`base + delta`), sin overflow ni
/// resultado negativo. Devuelve [`KError::InvalidArgument`] si se desborda o
/// caería por debajo de 0.
fn offset_add(base: u64, delta: i64) -> KResult<u64> {
    if delta >= 0 {
        base.checked_add(delta as u64).ok_or(KError::InvalidArgument)
    } else {
        // `unsigned_abs` da la magnitud correcta incluso para `i64::MIN`.
        base.checked_sub(delta.unsigned_abs()).ok_or(KError::InvalidArgument)
    }
}

impl OpenFile {
    /// Construye un `OpenFile` a partir de un inodo y los flags de apertura.
    ///
    /// Deriva `readable`/`writable`/`append` de los flags y aplica `TRUNC`
    /// (si es escribible) truncando el inodo a 0. Devuelve
    /// [`KError::InvalidArgument`] si no se pidió ningún modo de acceso.
    pub fn new(inode: Arc<dyn Inode>, flags: OpenFlags) -> KResult<Self> {
        let readable = flags.contains(OpenFlags::RDONLY);
        let writable = flags.contains(OpenFlags::WRONLY);
        if !readable && !writable {
            // Ningún bit de acceso: apertura sin sentido.
            return Err(KError::InvalidArgument);
        }
        let append = flags.contains(OpenFlags::APPEND);

        // Truncado en apertura: sólo tiene sentido si vamos a escribir.
        if flags.contains(OpenFlags::TRUNC) && writable {
            inode.truncate(0)?;
        }

        Ok(Self {
            inode,
            offset: 0,
            readable,
            writable,
            append,
        })
    }

    /// Lee desde la posición actual y avanza el offset. Requiere permiso de
    /// lectura.
    pub fn read(&mut self, buf: &mut [u8]) -> KResult<usize> {
        if !self.readable {
            return Err(KError::PermissionDenied);
        }
        let n = self.inode.read_at(self.offset, buf)?;
        self.offset = self
            .offset
            .checked_add(n as u64)
            .ok_or(KError::InvalidArgument)?;
        Ok(n)
    }

    /// Escribe en la posición actual y avanza el offset. Requiere permiso de
    /// escritura. En modo `append` reposiciona al final antes de escribir.
    pub fn write(&mut self, buf: &[u8]) -> KResult<usize> {
        if !self.writable {
            return Err(KError::PermissionDenied);
        }
        if self.append {
            self.offset = self.inode.size();
        }
        let n = self.inode.write_at(self.offset, buf)?;
        self.offset = self
            .offset
            .checked_add(n as u64)
            .ok_or(KError::InvalidArgument)?;
        Ok(n)
    }

    /// Reposiciona el offset y devuelve la nueva posición absoluta.
    pub fn seek(&mut self, pos: SeekFrom) -> KResult<u64> {
        let new = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(d) => offset_add(self.offset, d)?,
            SeekFrom::End(d) => offset_add(self.inode.size(), d)?,
        };
        self.offset = new;
        Ok(new)
    }
}
