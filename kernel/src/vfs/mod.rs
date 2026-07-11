//! Virtual File System: API de alto nivel del kernel ("todo-es-un-archivo").
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Reúne inodos ([`inode`]), descriptores ([`file`]), tabla de montajes
//! ([`mount`]) y `/dev` ([`devfs`]) en una API sencilla que consumen syscall y
//! shell: `open/close/read/write/seek/mkdir/unlink/readdir`.
//!
//! La tabla de descriptores es global al kernel (aún no hay procesos con tablas
//! propias, Fase 6). Toda operación devuelve `KResult`; ninguna panica.
#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;

pub mod devfs;
pub mod file;
pub mod inode;
pub mod mount;

pub use file::{Fd, OpenFile, OpenFlags, SeekFrom};
pub use inode::{DirEntry, FileSystem, Inode, InodeKind, VfsError};

/// Máximo de archivos abiertos simultáneos en la tabla global.
const MAX_OPEN_FILES: usize = 64;

/// Tabla global de descriptores de archivo del kernel.
struct FdTable {
    /// Ranuras: `None` = libre. El índice es el `Fd`.
    entries: Vec<Option<OpenFile>>,
}

impl FdTable {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Inserta un archivo abierto y devuelve su `Fd` (primera ranura libre).
    fn insert(&mut self, file: OpenFile) -> KResult<Fd> {
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(file);
                return Ok(i as Fd);
            }
        }
        let idx = self.entries.len();
        if idx >= MAX_OPEN_FILES {
            return Err(KError::TableFull);
        }
        self.entries.push(Some(file));
        Ok(idx as Fd)
    }

    /// Referencia mutable al archivo del descriptor `fd`.
    fn get_mut(&mut self, fd: Fd) -> KResult<&mut OpenFile> {
        if fd < 0 {
            return Err(KError::BadFd);
        }
        match self.entries.get_mut(fd as usize) {
            Some(Some(f)) => Ok(f),
            _ => Err(KError::BadFd),
        }
    }

    /// Cierra el descriptor `fd`, liberando su ranura.
    fn remove(&mut self, fd: Fd) -> KResult<()> {
        if fd < 0 {
            return Err(KError::BadFd);
        }
        match self.entries.get_mut(fd as usize) {
            Some(slot) if slot.is_some() => {
                *slot = None;
                Ok(())
            }
            _ => Err(KError::BadFd),
        }
    }
}

/// Tabla de descriptores global. Estática, protegida por `Mutex`.
static FD_TABLE: Mutex<FdTable> = Mutex::new(FdTable::new());

/// Inicializa el VFS. Las tablas de montaje y descriptores son estáticas y
/// arrancan vacías; aquí no hay nada que asignar todavía.
pub fn init() -> KResult<()> {
    Ok(())
}

/// Crea un inodo (`File` o `Dir`) en la ruta indicada, resolviendo el
/// directorio padre y delegando en `Inode::create`.
fn create_path(path: &str, kind: InodeKind) -> KResult<Arc<dyn Inode>> {
    let norm = mount::normalize(path)?;
    let (parent_path, name) = mount::split_parent(&norm)?;
    if name.len() > mount::MAX_NAME_LEN {
        return Err(KError::NameTooLong);
    }
    let parent = mount::resolve(parent_path)?;
    if parent.kind() != InodeKind::Dir {
        return Err(KError::NotADirectory);
    }
    parent.create(name, kind)
}

/// Abre `path` con los flags dados y devuelve un `Fd`.
///
/// Si la ruta no existe y `flags` contiene `CREATE`, se crea un fichero. `TRUNC`
/// y el modo de acceso se aplican al construir el [`OpenFile`].
pub fn open(path: &str, flags: OpenFlags) -> KResult<Fd> {
    let inode = match mount::resolve(path) {
        Ok(node) => node,
        Err(KError::NotFound) if flags.contains(OpenFlags::CREATE) => {
            create_path(path, InodeKind::File)?
        }
        Err(e) => return Err(e),
    };

    // Abrir un directorio para escritura no tiene sentido.
    if inode.kind() == InodeKind::Dir
        && flags.contains(OpenFlags::WRONLY)
    {
        return Err(KError::IsADirectory);
    }

    let open = OpenFile::new(inode, flags)?;
    let mut table = FD_TABLE.lock();
    table.insert(open)
}

/// Cierra un descriptor.
pub fn close(fd: Fd) -> KResult<()> {
    let mut table = FD_TABLE.lock();
    table.remove(fd)
}

/// Lee del descriptor `fd`; devuelve bytes leídos.
pub fn read(fd: Fd, buf: &mut [u8]) -> KResult<usize> {
    let mut table = FD_TABLE.lock();
    let file = table.get_mut(fd)?;
    file.read(buf)
}

/// Escribe en el descriptor `fd`; devuelve bytes escritos.
pub fn write(fd: Fd, buf: &[u8]) -> KResult<usize> {
    let mut table = FD_TABLE.lock();
    let file = table.get_mut(fd)?;
    file.write(buf)
}

/// Reposiciona el descriptor `fd`; devuelve la nueva posición absoluta.
pub fn seek(fd: Fd, pos: SeekFrom) -> KResult<u64> {
    let mut table = FD_TABLE.lock();
    let file = table.get_mut(fd)?;
    file.seek(pos)
}

/// Crea un directorio en `path`. Error si ya existe.
pub fn mkdir(path: &str) -> KResult<()> {
    if mount::resolve(path).is_ok() {
        return Err(KError::AlreadyExists);
    }
    create_path(path, InodeKind::Dir)?;
    Ok(())
}

/// Elimina la entrada (fichero o directorio vacío) en `path`.
pub fn unlink(path: &str) -> KResult<()> {
    let norm = mount::normalize(path)?;
    let (parent_path, name) = mount::split_parent(&norm)?;
    let parent = mount::resolve(parent_path)?;
    parent.unlink(name)
}

/// Lista el contenido del directorio en `path`.
pub fn readdir(path: &str) -> KResult<Vec<DirEntry>> {
    let dir = mount::resolve(path)?;
    if dir.kind() != InodeKind::Dir {
        return Err(KError::NotADirectory);
    }
    let mut out = Vec::new();
    let mut index: usize = 0;
    loop {
        match dir.readdir(index)? {
            Some(entry) => {
                out.push(entry);
                index = index.checked_add(1).ok_or(KError::InvalidArgument)?;
            }
            None => break,
        }
    }
    Ok(out)
}

/// Monta un FS en `path` (reexporta [`mount::mount`]).
pub fn mount(path: &str, fs: Arc<dyn FileSystem>) -> KResult<()> {
    mount::mount(path, fs)
}

/// Desmonta el FS en `path` (reexporta [`mount::unmount`]).
pub fn unmount(path: &str) -> KResult<()> {
    mount::unmount(path)
}
