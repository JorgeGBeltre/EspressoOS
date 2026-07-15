#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

pub mod devfs;
pub mod file;
pub mod inode;
pub mod mount;
pub mod pipe;
pub mod socket;

pub use file::{Fd, OpenFile, OpenFlags, SeekFrom};
pub use inode::{DirEntry, FileSystem, Inode, InodeKind, VfsError};

const MAX_OPEN_FILES: usize = 64;

#[derive(Clone)]
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

use crate::scheduler::process::Pid;
use alloc::collections::BTreeMap;

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

    if inode.kind() == InodeKind::Dir && flags.contains(OpenFlags::WRONLY) {
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

/// Writes the post-I/O offset back into the fd table.
///
/// The table was unlocked for the duration of the I/O, so `fd` may have been
/// closed and reused meanwhile. `Arc::ptr_eq` only rules out the coarse case:
/// the slot now holds a *different* inode, so the offset is dropped rather than
/// applied to someone else's file.
///
/// It is NOT open-file identity. ramfs hands back the same cached Arc for a path
/// (fs/ramfs.rs, `lookup` returns `node.clone()`) and dup2 shares the Arc across
/// fds, so closing and reopening the same ramfs file is indistinguishable here
/// and would inherit the stale offset. Making that exact needs a generation
/// counter on OpenFile, not a pointer compare. It is unreachable today because
/// `FdTable::insert` never hands out an occupied slot and no task closes another
/// task's fd. POSIX permits a concurrent close() to lose the update anyway.
fn commit_offset(pid: Pid, fd: Fd, inode: &Arc<dyn Inode>, new_offset: u64) {
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = match tables.get_mut(&pid) {
        Some(t) => t,
        None => return,
    };
    let file = match table.get_mut(fd) {
        Ok(f) => f,
        Err(_) => return,
    };
    if Arc::ptr_eq(&file.inode, inode) {
        file.offset = new_offset;
    }
}

// read/write release the fd table guard before touching the inode.
//
// `Mutex::lock` disables interrupts for the whole lifetime of the guard, so
// calling read_at/write_at underneath it means a blocking inode (a pipe on an
// empty buffer, a socket) parks the task while still holding the global fd lock
// with interrupts off. Every other task then spins on that lock forever and the
// timer cannot preempt anyone: the kernel wedges. So: snapshot the fd, unlock,
// do the I/O with no lock held, then re-credit the offset.
//
// The price is that the offset read-modify-write is no longer atomic with the
// I/O for any file, not just the append case below: the global fd lock used to
// serialize every read/write in the system against each other. Two tasks sharing
// one fd can now lose an offset update. Nothing shares a seekable fd today (the
// inherited 0/1/2 are console devices that ignore the offset entirely).

pub fn read(fd: Fd, buf: &mut [u8]) -> KResult<usize> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);

    let (inode, offset, readable) = {
        let mut tables = PROCESS_FD_TABLES.lock();
        let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
        let file = table.get_mut(fd)?;
        (file.inode.clone(), file.offset, file.readable)
    };

    if !readable {
        return Err(KError::PermissionDenied);
    }

    let n = inode.read_at(offset, buf)?;

    let new_offset = offset
        .checked_add(n as u64)
        .ok_or(KError::InvalidArgument)?;
    commit_offset(pid, fd, &inode, new_offset);
    Ok(n)
}

pub fn write(fd: Fd, buf: &[u8]) -> KResult<usize> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);

    let (inode, offset, writable, append) = {
        let mut tables = PROCESS_FD_TABLES.lock();
        let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);
        let file = table.get_mut(fd)?;
        (file.inode.clone(), file.offset, file.writable, file.append)
    };

    if !writable {
        return Err(KError::PermissionDenied);
    }

    // size() is an inode call, so it belongs out here with the rest of the I/O
    // rather than under the guard. The cost is that append is no longer atomic
    // against a concurrent appender to the same inode: the global fd lock used
    // to serialize size()+write_at for everyone. Closing that needs an
    // inode-level append, not the fd table.
    let offset = if append { inode.size() } else { offset };

    let n = inode.write_at(offset, buf)?;

    let new_offset = offset
        .checked_add(n as u64)
        .ok_or(KError::InvalidArgument)?;
    commit_offset(pid, fd, &inode, new_offset);
    Ok(n)
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

/// Installs a fresh fd table for `pid` with 0/1/2 all bound to `inode`.
///
/// This is an unconditional `insert`, deliberately not `entry().or_insert_with`:
/// every other fd function falls back to `FdTable::new_process_table`, which is
/// hardcoded to /dev/console. If a session's task reached any of them before it
/// was seeded, its stdio would silently land on the serial port instead of its
/// own channel -- and an SSH session would type into the UART. So the task must
/// be created blocked, seeded here, and only then unblocked.
pub fn seed_fd_table(pid: Pid, inode: Arc<dyn Inode>) -> KResult<()> {
    let stdin = OpenFile::new(inode.clone(), OpenFlags::RDONLY)?;
    let stdout = OpenFile::new(inode.clone(), OpenFlags::WRONLY)?;
    let stderr = OpenFile::new(inode, OpenFlags::WRONLY)?;

    let mut table = FdTable::new();
    table.insert(stdin)?;
    table.insert(stdout)?;
    table.insert(stderr)?;

    PROCESS_FD_TABLES.lock().insert(pid, table);
    Ok(())
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

pub fn dup2(oldfd: Fd, newfd: Fd) -> KResult<Fd> {
    if oldfd < 0 || newfd < 0 || newfd >= MAX_OPEN_FILES as Fd {
        return Err(KError::BadFd);
    }
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);

    let open_file = match table.entries.get(oldfd as usize) {
        Some(Some(f)) => f.clone(),
        _ => return Err(KError::BadFd),
    };

    if oldfd == newfd {
        return Ok(newfd);
    }

    while table.entries.len() <= newfd as usize {
        table.entries.push(None);
    }

    table.entries[newfd as usize] = Some(open_file);
    Ok(newfd)
}

/// Duplicates `fd` onto the lowest free slot, like dup(2).
pub fn dup(fd: Fd) -> KResult<Fd> {
    if fd < 0 {
        return Err(KError::BadFd);
    }
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.entry(pid).or_insert_with(FdTable::new_process_table);

    let open_file = match table.entries.get(fd as usize) {
        Some(Some(f)) => f.clone(),
        _ => return Err(KError::BadFd),
    };
    table.insert(open_file)
}

pub fn clone_fd_table(parent_pid: Pid, child_pid: Pid) {
    let mut tables = PROCESS_FD_TABLES.lock();
    let parent_table = tables.get(&parent_pid).cloned();
    if let Some(t) = parent_table {
        tables.insert(child_pid, t);
    }
}
