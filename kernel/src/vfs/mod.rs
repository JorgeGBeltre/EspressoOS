#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::sync::Mutex;

pub mod devfs;
pub mod file;
pub mod inode;
pub mod mount;
pub mod pipe;
pub mod socket;

pub use file::{Fd, OpenFile, OpenFlags, SeekFrom};
pub use inode::{DirEntry, FileSystem, Inode, InodeKind, VfsError};

const MAX_OPEN_FILES: usize = 64;

struct FdTable {

    entries: Vec<Option<OpenFile>>,
}

impl FdTable {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

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

    fn get_mut(&mut self, fd: Fd) -> KResult<&mut OpenFile> {
        if fd < 0 {
            return Err(KError::BadFd);
        }
        match self.entries.get_mut(fd as usize) {
            Some(Some(f)) => Ok(f),
            _ => Err(KError::BadFd),
        }
    }

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

impl FdTable {
    pub fn new_process_table() -> Self {
        let mut table = Self::new();
        if let Ok(inode) = mount::resolve("/dev/console") {
            if let Ok(file_in) = OpenFile::new(inode.clone(), OpenFlags::RDONLY) {
                let _ = table.insert(file_in);
            }
            if let Ok(file_out1) = OpenFile::new(inode.clone(), OpenFlags::WRONLY) {
                let _ = table.insert(file_out1);
            }
            if let Ok(file_out2) = OpenFile::new(inode.clone(), OpenFlags::WRONLY) {
                let _ = table.insert(file_out2);
            }
        }
        table
    }
}

use alloc::collections::BTreeMap;
use crate::scheduler::process::Pid;

static PROCESS_FD_TABLES: Mutex<BTreeMap<Pid, FdTable>> = Mutex::new(BTreeMap::new());

pub fn init() -> KResult<()> {
    Ok(())
}

pub fn cleanup_process_fds(pid: Pid) {
    let mut tables = PROCESS_FD_TABLES.lock();
    tables.remove(&pid);
}

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

pub fn open(path: &str, flags: OpenFlags) -> KResult<Fd> {
    let inode = match mount::resolve(path) {
        Ok(node) => node,
        Err(KError::NotFound) if flags.contains(OpenFlags::CREATE) => {
            create_path(path, InodeKind::File)?
        }
        Err(e) => return Err(e),
    };

    if inode.kind() == InodeKind::Dir
        && flags.contains(OpenFlags::WRONLY)
    {
        return Err(KError::IsADirectory);
    }

    let open = OpenFile::new(inode, flags)?;
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    table.insert(open)
}

pub fn close(fd: Fd) -> KResult<()> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    table.remove(fd)
}

pub fn read(fd: Fd, buf: &mut [u8]) -> KResult<usize> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    let file = table.get_mut(fd)?;
    file.read(buf)
}

pub fn write(fd: Fd, buf: &[u8]) -> KResult<usize> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    let file = table.get_mut(fd)?;
    file.write(buf)
}

pub fn seek(fd: Fd, pos: SeekFrom) -> KResult<u64> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    let file = table.get_mut(fd)?;
    file.seek(pos)
}

pub fn mkdir(path: &str) -> KResult<()> {
    if mount::resolve(path).is_ok() {
        return Err(KError::AlreadyExists);
    }
    create_path(path, InodeKind::Dir)?;
    Ok(())
}

pub fn unlink(path: &str) -> KResult<()> {
    let norm = mount::normalize(path)?;
    let (parent_path, name) = mount::split_parent(&norm)?;
    let parent = mount::resolve(parent_path)?;
    parent.unlink(name)
}

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

pub fn mount(path: &str, fs: Arc<dyn FileSystem>) -> KResult<()> {
    mount::mount(path, fs)
}

pub fn unmount(path: &str) -> KResult<()> {
    mount::unmount(path)
}

pub fn create_pipe() -> KResult<(Fd, Fd)> {
    let (read_inode, write_inode) = pipe::create_pipe(4096);
    let read_file = OpenFile::new(read_inode, OpenFlags::RDONLY)?;
    let write_file = OpenFile::new(write_inode, OpenFlags::WRONLY)?;
    
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    
    let r_fd = table.insert(read_file)?;
    match table.insert(write_file) {
        Ok(w_fd) => Ok((r_fd, w_fd)),
        Err(e) => {
            let _ = table.remove(r_fd);
            Err(e)
        }
    }
}

pub fn insert_open_file(open: OpenFile) -> KResult<Fd> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    table.insert(open)
}

pub fn get_inode(fd: Fd) -> KResult<Arc<dyn Inode>> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    let open_file = table.get_mut(fd)?;
    Ok(open_file.inode.clone())
}

pub fn remove_fd(fd: Fd) -> KResult<()> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
    table.remove(fd)?;
    Ok(())
}
