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

// FdTable::new_process_table lived here: it built a table with 0/1/2 bound to
// /dev/console, and eleven fd functions called it through
// `tables.entry(pid).or_insert_with(...)`.
//
// It is gone rather than fixed, because as long as it existed the bug could be
// written again. It did two bad things at once.
//
// It resolved a path with PROCESS_FD_TABLES held -- the only place in the VFS that
// did. `open` has the right shape: resolve at the top, lock afterwards. That was inert
// only because "/dev/console" is an absolute literal, and it stops being inert the
// moment resolution can consult the caller's cwd: it would then take SCHED and
// PROCESS_TABLE under the fd lock, while register_process already holds PROCESS_TABLE
// across a call to clone_fd_table, which takes PROCESS_FD_TABLES. That is the cycle,
// and on this Mutex a cycle is a silent wedge with the interrupts off, not a panic.
//
// And it made "this process has no fd table" unrepresentable. A caller with no
// process, or one already reaped, silently got a brand-new table wired to the serial
// port instead of an error. seed_fd_table below has carried a comment about that
// hazard for a while; it is cheaper to delete the hazard than to keep documenting it.

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

/// `pid`'s fd table, or BadFd if it has none.
///
/// A missing table is an error, never something to conjure. Every fd of a process
/// that has one was put there deliberately: seed_fd_table at session start,
/// clone_fd_table at fork, or an open. So the only callers that can miss are a task
/// with no process and no seeded table, or a process already reaped -- and for both,
/// every fd they could name is invalid, which is exactly what BadFd says.
///
/// The kernel's own pidless tasks reach this through `unwrap_or(0)`; pid 0's table is
/// seeded at boot in main.rs, next to the mounts, so that "no table" cannot mean "not
/// booted yet".
fn table_of(tables: &mut BTreeMap<Pid, FdTable>, pid: Pid) -> KResult<&mut FdTable> {
    tables.get_mut(&pid).ok_or(KError::BadFd)
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
    let table = table_of(&mut tables, pid)?;
    table.insert(open)
}

pub fn close(fd: Fd) -> KResult<()> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = table_of(&mut tables, pid)?;
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
        let table = table_of(&mut tables, pid)?;
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
        let table = table_of(&mut tables, pid)?;
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
    let table = table_of(&mut tables, pid)?;
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
    // POSIX: unlinking "." or ".." shall fail. The check has to happen on the path AS
    // WRITTEN, before normalize touches it, and that is the whole reason it could not
    // exist until now.
    //
    // normalize collapses "." and pops ".." lexically. A moment later `rm .` from
    // /tmp/x is the string "/tmp/x" -- indistinguishable from a deliberate
    // `rm /tmp/x` typed from somewhere else, and there is nothing left to refuse. The
    // shell used to do that collapsing itself, in its own resolve(), so unlink only
    // ever saw the aftermath. `rm .` silently deleted the caller's working directory,
    // leaving `pwd` naming a directory that no longer existed and no chdir syscall to
    // escape with.
    //
    // Deleting the shell's resolve() is what let the raw "." reach this line.
    //
    // Note what this does NOT forbid: `rm /tmp/x` from inside /tmp/x. POSIX allows it,
    // and the path names the directory rather than gesturing at it.
    // trim_end_matches first. Without it a single trailing slash walks straight past
    // this: "./" splits to ["", "."], so rsplit's first item is the empty string, not
    // ".", and `rm ./` deleted the cwd exactly as `rm .` used to. The guard has to see
    // the last NAME, and a trailing slash is punctuation, not a name.
    let last = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    if last == "." || last == ".." {
        return Err(KError::InvalidArgument);
    }

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

    if let Ok(children) = mount::child_mounts(path) {
        for (name, kind) in children {
            if !out.iter().any(|e| e.name == name) {
                out.push(DirEntry {
                    name,
                    kind,
                    ino: 1,
                });
            }
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
    let table = table_of(&mut tables, pid)?;

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
/// A session's task must still be created blocked, seeded here, and only then
/// unblocked -- but the reason has changed, and improved. It used to be that reaching
/// any other fd function first would silently mint a table hardcoded to /dev/console,
/// so an SSH session that raced its own seeding would have typed into the UART. That
/// fallback is gone: the race now yields BadFd, which is loud and harmless. The order
/// still matters, it just no longer has a wrong answer.
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
    let table = table_of(&mut tables, pid)?;
    table.insert(open)
}

pub fn get_inode(fd: Fd) -> KResult<Arc<dyn Inode>> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = table_of(&mut tables, pid)?;
    let open_file = table.get_mut(fd)?;
    Ok(open_file.inode.clone())
}

pub fn remove_fd(fd: Fd) -> KResult<()> {
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = table_of(&mut tables, pid)?;
    table.remove(fd)?;
    Ok(())
}

pub fn dup2(oldfd: Fd, newfd: Fd) -> KResult<Fd> {
    if oldfd < 0 || newfd < 0 || newfd >= MAX_OPEN_FILES as Fd {
        return Err(KError::BadFd);
    }
    let pid = crate::scheduler::process::get_current_pid().unwrap_or(0);
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = table_of(&mut tables, pid)?;

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
    let table = table_of(&mut tables, pid)?;

    let open_file = match table.entries.get(fd as usize) {
        Some(Some(f)) => f.clone(),
        _ => return Err(KError::BadFd),
    };
    table.insert(open_file)
}

// dup2/close aimed at another process's table.
//
// There is no fork here. In Unix the child dup2s itself between fork and exec;
// this child does not run until unblock_task, so whoever spawns it has to arrange
// its stdio from the outside. Both take a pid rather than using the caller's table.

/// dup2 inside `pid`'s table.
pub fn dup2_in(pid: Pid, oldfd: Fd, newfd: Fd) -> KResult<Fd> {
    if oldfd < 0 || newfd < 0 || newfd >= MAX_OPEN_FILES as Fd {
        return Err(KError::BadFd);
    }
    let mut tables = PROCESS_FD_TABLES.lock();
    let table = tables.get_mut(&pid).ok_or(KError::BadFd)?;

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

/// close inside `pid`'s table. Missing fds are not an error: callers close a whole
/// set of pipe ends without tracking which of them a given child ever had.
pub fn close_in(pid: Pid, fd: Fd) {
    if fd < 0 {
        return;
    }
    let mut tables = PROCESS_FD_TABLES.lock();
    if let Some(table) = tables.get_mut(&pid) {
        let _ = table.remove(fd);
    }
}

pub fn clone_fd_table(parent_pid: Pid, child_pid: Pid) {
    let mut tables = PROCESS_FD_TABLES.lock();
    let parent_table = tables.get(&parent_pid).cloned();
    if let Some(t) = parent_table {
        tables.insert(child_pid, t);
    }
}
