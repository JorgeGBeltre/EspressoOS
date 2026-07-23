#![allow(dead_code)]

use super::table::Syscall;
use crate::prelude::*;
use crate::vfs::{Fd, InodeKind, OpenFlags, SeekFrom};

pub fn dispatch(
    num: usize,
    args: &[usize],
    frame: *mut esp_hal::xtensa_lx_rt::exception::Context,
) -> isize {
    let sc = match Syscall::from_usize(num) {
        Some(s) => s,
        None => return KError::NotSupported.as_errno(),
    };

    match sc {
        Syscall::Read => sys_read(args),
        Syscall::Write => sys_write(args),
        Syscall::Open => sys_open(args),
        Syscall::Close => sys_close(args),
        Syscall::Ioctl => sys_ioctl(args),
        Syscall::Exit => sys_exit(args),
        Syscall::Spawn => sys_spawn(args),
        Syscall::Wait => sys_wait(args),
        Syscall::Seek => sys_seek(args),
        Syscall::Mkdir => sys_mkdir(args),
        Syscall::Unlink => sys_unlink(args),
        Syscall::Readdir => sys_readdir(args),
        Syscall::UptimeMs => sys_uptime_ms(args),
        Syscall::Sbrk => sys_sbrk(args),
        Syscall::Yield => sys_yield(args),
        Syscall::Signal => sys_sigaction(args),
        Syscall::Kill => sys_kill(args),
        Syscall::Sigreturn => sys_sigreturn(args, frame),
        Syscall::Socket => sys_socket(args),
        Syscall::Bind => sys_bind(args),
        Syscall::Listen => sys_listen(args),
        Syscall::Accept => sys_accept(args),
        Syscall::Connect => sys_connect(args),
        Syscall::GetTimeOfDay => sys_gettimeofday(args),
        Syscall::SetTimeOfDay => sys_settimeofday(args),
        Syscall::OtaState => sys_ota_state(args),
        Syscall::Pipe => sys_pipe(args),
        Syscall::Dup2 => sys_dup2(args),
        Syscall::Chdir => sys_chdir(args),
        Syscall::Getcwd => sys_getcwd(args),
    }
}

fn sys_dup2(args: &[usize]) -> isize {
    let oldfd = arg(args, 0) as i32;
    let newfd = arg(args, 1) as i32;
    match crate::vfs::dup2(oldfd, newfd) {
        Ok(fd) => fd as isize,
        Err(e) => e.as_errno(),
    }
}

fn sys_pipe(args: &[usize]) -> isize {
    let out = match unsafe { user_slice_mut(arg(args, 0), 8) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    match crate::vfs::create_pipe() {
        Ok((r, w)) => {
            out[0..4].copy_from_slice(&(r as i32).to_le_bytes());
            out[4..8].copy_from_slice(&(w as i32).to_le_bytes());
            0
        }
        Err(e) => e.as_errno(),
    }
}

#[inline]
fn arg(args: &[usize], i: usize) -> usize {
    match args.get(i) {
        Some(&v) => v,
        None => 0,
    }
}

/// Rejects a pointer a user process has no business handing us.
///
/// The boundary is the MODE, not the process: a kernel task is trusted with any
/// address, which is what lets the shell pass `&mut status` from its own stack to
/// sys_wait. A user task gets exactly two regions:
///
///   * **its own stack** -- a kernel heap allocation, not part of its slot, and
///     where every `&mut buf` in the userland lives. Omitting it would reject every
///     `read()` there is.
///   * **its own data slot** -- globals, `.rodata`, `.bss` and argv. Not the text
///     slot: nothing in a program points a data pointer at its own code.
///
/// Anything else -- another process's slot, the kernel's own structures, a
/// peripheral register -- is a Fault. Until this existed, `sys_wait` would write an
/// exit code to any address it was given and `user_slice` only rejected null, which
/// was an arbitrary kernel write reachable from userland.
/// The end of whichever region `ptr` belongs to, or Fault if it belongs to none.
///
/// The end matters as much as the membership: it is the only honest bound for
/// walking a NUL-terminated string a process handed us. A fixed cap is wrong in
/// both directions -- 4 KB would let a string near the top of the stack be read
/// past the stack's end, and would reject a legitimate 5 KB one in the data slot.
///
/// Returns None for a kernel task, which is trusted and has no region.
fn user_region_end(ptr: usize) -> KResult<Option<usize>> {
    if ptr == 0 {
        return Err(KError::Fault);
    }
    if !crate::scheduler::current_task_is_user() {
        return Ok(None);
    }
    if let Some((lo, hi)) = crate::scheduler::current_stack_range() {
        if ptr >= lo && ptr < hi {
            return Ok(Some(hi));
        }
    }
    if let Some(slot) = crate::scheduler::process::current_slot() {
        let lo = crate::mm::psram_exec::slot_data(slot) as usize;
        let hi = lo + crate::mm::psram_exec::SLOT_SIZE as usize;
        if ptr >= lo && ptr < hi {
            return Ok(Some(hi));
        }
    }
    Err(KError::Fault)
}

pub(crate) fn validate_user(ptr: usize, len: usize) -> KResult<()> {
    if len == 0 {
        return Ok(());
    }
    let end = ptr.checked_add(len).ok_or(KError::Fault)?;
    match user_region_end(ptr)? {
        None => Ok(()),
        Some(hi) if end <= hi => Ok(()),
        Some(_) => Err(KError::Fault),
    }
}

/// A kernel task has no region, so its strings still need SOME bound -- trusted is
/// not the same as infinite, and a missing NUL would spin forever.
const KERNEL_STR_MAX: usize = 4096;

/// Length of a NUL-terminated string a process handed us, without reading past the
/// region it lives in.
pub(crate) fn user_strnlen(ptr: usize) -> KResult<usize> {
    let hi = user_region_end(ptr)?.unwrap_or(ptr.saturating_add(KERNEL_STR_MAX));
    let mut n = 0usize;
    while ptr + n < hi {
        if unsafe { *((ptr + n) as *const u8) } == 0 {
            return Ok(n);
        }
        n += 1;
    }
    // Ran to the end of the region without finding one.
    Err(KError::Fault)
}

unsafe fn user_slice<'a>(ptr: usize, len: usize) -> KResult<&'a [u8]> {
    if len == 0 {
        return Ok(&[]);
    }
    validate_user(ptr, len)?;
    Ok(core::slice::from_raw_parts(ptr as *const u8, len))
}

unsafe fn user_slice_mut<'a>(ptr: usize, len: usize) -> KResult<&'a mut [u8]> {
    if len == 0 {
        return Ok(&mut []);
    }
    validate_user(ptr, len)?;
    Ok(core::slice::from_raw_parts_mut(ptr as *mut u8, len))
}

unsafe fn user_str<'a>(ptr: usize, len: usize) -> KResult<&'a str> {
    let bytes = user_slice(ptr, len)?;
    core::str::from_utf8(bytes).map_err(|_| KError::InvalidArgument)
}

#[inline]
fn ret_usize(r: KResult<usize>) -> isize {
    match r {
        Ok(n) => core::cmp::min(n, isize::MAX as usize) as isize,
        Err(e) => e.as_errno(),
    }
}

#[inline]
fn ret_unit(r: KResult<()>) -> isize {
    match r {
        Ok(()) => 0,
        Err(e) => e.as_errno(),
    }
}

#[inline]
const fn kind_to_u8(kind: InodeKind) -> u8 {
    match kind {
        InodeKind::File => 1,
        InodeKind::Dir => 2,
        InodeKind::Device => 3,
        InodeKind::Symlink => 4,
    }
}

fn sys_read(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let buf = match unsafe { user_slice_mut(arg(args, 1), arg(args, 2)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_usize(crate::vfs::read(fd, buf))
}

fn sys_write(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let buf = match unsafe { user_slice(arg(args, 1), arg(args, 2)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_usize(crate::vfs::write(fd, buf))
}

fn sys_open(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    let flags = OpenFlags(arg(args, 2) as u32);
    match crate::vfs::open(path, flags) {
        Ok(fd) => fd as isize,
        Err(e) => e.as_errno(),
    }
}

fn sys_close(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    ret_unit(crate::vfs::close(fd))
}

fn sys_ioctl(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let cmd = arg(args, 1) as u32;
    let val = arg(args, 2);
    match crate::vfs::get_inode(fd) {
        Ok(inode) => match inode.ioctl(cmd, val) {
            Ok(ret) => ret as isize,
            Err(e) => e.as_errno(),
        },
        Err(e) => e.as_errno(),
    }
}

fn sys_exit(args: &[usize]) -> isize {
    let code = arg(args, 0) as i32;

    let current_tid = crate::scheduler::current();

    let parent_to_wake = {
        let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();
        let current_pid = pt.pid_of_tid(current_tid);

        let parent_pid = match current_pid.and_then(|pid| pt.table.get_mut(&pid)) {
            Some(proc) => {
                proc.state = crate::scheduler::process::ProcessState::Zombie;
                proc.exit_code = code;
                proc.parent_pid
            }
            None => None,
        };
        parent_pid
            .and_then(|p| pt.table.get(&p))
            .map(|p| p.main_task)
    };

    if let Some(parent_tid) = parent_to_wake {
        crate::scheduler::unblock_task(parent_tid);
    }

    crate::scheduler::mark_zombie(code);
    crate::scheduler::set_need_resched();
    0
}

/// Most arguments a process can hand `spawn`.
///
/// The walk terminates without this: `user_argv` cannot step outside the caller's
/// region, and `write_argv` refuses a blob that will not fit a 16 KB slot. The limit
/// is here because the Vec<String> is built BEFORE either of those bites -- a
/// thousand pointers to short strings would allocate a thousand kernel Strings and
/// only then be told the blob was too big.
const MAX_ARGC: usize = 64;

/// Copies a NULL-terminated `*const *const u8` out of the calling process.
///
/// Every pointer in it belongs to the caller and is checked as such, the same way any
/// other pointer crossing this boundary is. The strings are copied rather than
/// borrowed because the blob they end up in is built in a DIFFERENT slot -- the
/// child's -- after `load_elf` has run.
fn user_argv(argv_ptr: usize) -> KResult<Vec<String>> {
    if argv_ptr == 0 {
        return Ok(Vec::new());
    }
    // An unaligned char** is a bug in the caller, but `l32i` on an unaligned address
    // traps, which would turn that bug into a kernel exception. Refuse it here rather
    // than take the fault.
    if argv_ptr & 3 != 0 {
        return Err(KError::Fault);
    }

    let mut out: Vec<String> = Vec::new();
    let mut p = argv_ptr;
    loop {
        if out.len() == MAX_ARGC {
            return Err(KError::NoSpace);
        }
        validate_user(p, 4)?;
        let s = unsafe { *(p as *const u32) } as usize;
        if s == 0 {
            return Ok(out);
        }
        // user_strnlen bounds itself to the region `s` lives in, so a slice of that
        // length starting there cannot reach past it. Re-validating would run the same
        // region check twice and could not fail differently.
        let len = user_strnlen(s)?;
        let bytes = unsafe { core::slice::from_raw_parts(s as *const u8, len) };
        out.push(String::from(
            core::str::from_utf8(bytes).map_err(|_| KError::InvalidArgument)?,
        ));
        p += 4;
    }
}

/// Starts a program from an image on disk.
///
///     arg 0,1 = path (ptr, len) in the caller's memory
///     arg 2   = argv: NULL-terminated *const *const u8 in the caller's memory, or 0
///
/// argv is copied into the child's slot, which is what execve does and for the same
/// reason: the array the parent holds is in the parent's memory, and the child has
/// its own.
///
/// The second form of this call -- a raw entry point, taken when arg 2 was non-zero
/// -- is gone. Nothing used it; all five callers in the tree passed 0. What it did
/// was spawn a task at an address of userland's choosing with `is_user = false`,
/// which is arbitrary kernel execution and, because validate_user trusts any task
/// that is not a user task, also a switch that turns the pointer checks off. It could
/// not be repaired by validating the entry point: there is no address a user process
/// has any business jumping to as the kernel.
fn sys_spawn(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };

    // Copied out before anything is allocated: a bad argv never reaches a slot.
    let mut argv = match user_argv(arg(args, 2)) {
        Ok(v) => v,
        Err(e) => return e.as_errno(),
    };
    // A child always gets an argv[0]. `_start` unpacks the blob unconditionally, so
    // "no arguments" has to mean [path] -- passing 0 is what made every program
    // sys_spawn started take a LoadProhibited on its first instruction.
    if argv.is_empty() {
        argv.push(String::from(path));
    }

    let loaded = match crate::fs::elf::load_elf(path) {
        Ok(l) => l,
        Err(e) => return e.as_errno(),
    };

    let refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let argv_ptr = match crate::fs::elf::write_argv(&loaded, &refs) {
        Ok(p) => p,
        Err(e) => {
            // Nothing owns the image yet, so the slot is ours to drop.
            crate::mm::psram_exec::slot_free(loaded.slot);
            return e.as_errno();
        }
    };

    let entry: fn(usize) = unsafe { core::mem::transmute(loaded.entry as usize) };
    match crate::scheduler::spawn_blocked(path, entry, argv_ptr, layout::DEFAULT_STACK_SIZE, 10, true)
    {
        Ok(tid) => {
            // The slot goes on the process: reaping it is what returns the slot to
            // the pool.
            let pid =
                crate::scheduler::process::register_process(path, tid, true, Some(loaded.slot));
            crate::scheduler::unblock_task(tid);
            pid as isize
        }
        Err(e) => {
            crate::mm::psram_exec::slot_free(loaded.slot);
            e.as_errno()
        }
    }
}

fn sys_wait(args: &[usize]) -> isize {
    let status_ptr = arg(args, 0) as *mut i32;
    // The write that started all this: sys_wait used to store the exit code at
    // whatever address it was handed, checking only for null. A null pointer is a
    // legal "I don't want the status", so it is allowed through here and skipped at
    // the store.
    if !status_ptr.is_null() {
        if let Err(e) = validate_user(status_ptr as usize, core::mem::size_of::<i32>()) {
            return e.as_errno();
        }
    }
    let current_tid = crate::scheduler::current();

    let current_pid = match crate::scheduler::process::PROCESS_TABLE
        .lock()
        .pid_of_tid(current_tid)
    {
        Some(pid) => pid,
        None => return KError::NotFound.as_errno(),
    };

    if !crate::scheduler::process::has_children(current_pid) {
        return KError::NotFound.as_errno();
    }

    // Detaching the child belongs to process.rs: it was the one place outside that
    // module that removed from PROCESS_TABLE, which is what made the tid->pid map an
    // invariant kept by discipline rather than by construction.
    let reaped = crate::scheduler::process::take_zombie_child(current_pid);

    if let Some((child_pid, code, slot)) = reaped {
        if !status_ptr.is_null() {
            unsafe {
                *status_ptr = code;
            }
        }

        // The image lived in a PSRAM slot, not on the heap: hand the slot back
        // instead of the dealloc this used to do.
        if let Some(s) = slot {
            crate::mm::psram_exec::slot_free(s);
        }
        crate::vfs::cleanup_process_fds(child_pid);
        return child_pid as isize;
    }

    crate::scheduler::block_current_noswitch();
    crate::scheduler::set_restart_syscall();
    0
}

fn sys_seek(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let off = arg(args, 1);
    let whence = arg(args, 2);

    let pos = match whence {
        0 => SeekFrom::Start(off as u64),
        1 => SeekFrom::Current(off as isize as i64),
        2 => SeekFrom::End(off as isize as i64),
        _ => return KError::InvalidArgument.as_errno(),
    };
    match crate::vfs::seek(fd, pos) {
        Ok(n) => core::cmp::min(n, isize::MAX as u64) as isize,
        Err(e) => e.as_errno(),
    }
}

fn sys_mkdir(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_unit(crate::vfs::mkdir(path))
}

fn sys_unlink(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    ret_unit(crate::vfs::unlink(path))
}

fn sys_chdir(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    // Existencia + que sea directorio, ANTES de fijar el cwd. cwd_set sólo valida
    // sintácticamente (que normalize devuelva una ruta absoluta); sin este paso,
    // `cd /noexiste` "tendría éxito" y toda ruta relativa posterior resolvería contra
    // un directorio que no está. POSIX chdir falla ENOENT/ENOTDIR; esto lo replica.
    // resolve() ya resuelve la ruta contra el cwd del que llama, igual que cwd_set la
    // normalizará después: las dos ven la misma ruta.
    match crate::vfs::mount::resolve(path) {
        Ok(inode) => {
            if inode.kind() != crate::vfs::InodeKind::Dir {
                return KError::NotADirectory.as_errno();
            }
        }
        Err(e) => return e.as_errno(),
    }
    ret_unit(crate::scheduler::process::cwd_set(path))
}

fn sys_getcwd(args: &[usize]) -> isize {
    let buf = match unsafe { user_slice_mut(arg(args, 0), arg(args, 1)) } {
        Ok(b) => b,
        Err(e) => return e.as_errno(),
    };
    let cwd = crate::scheduler::process::cwd_get();
    let bytes = cwd.as_bytes();
    // Este kernel no tiene ERANGE en su set de errno; un buffer demasiado pequeño es
    // un bug del que llama y se mapea a InvalidArgument. El wrapper de userland
    // dimensiona su buffer muy por encima de cualquier ruta real (rutas cortas y
    // absolutas), así que esto no dispara en la práctica. (Desviación consciente del
    // spec, que dice -ERANGE.)
    if bytes.len() > buf.len() {
        return KError::InvalidArgument.as_errno();
    }
    buf[..bytes.len()].copy_from_slice(bytes);
    bytes.len() as isize
}

fn sys_readdir(args: &[usize]) -> isize {
    let path = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    let out = match unsafe { user_slice_mut(arg(args, 2), arg(args, 3)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };

    let entries = match crate::vfs::readdir(path) {
        Ok(v) => v,
        Err(e) => return e.as_errno(),
    };

    let mut pos: usize = 0;
    for e in entries.iter() {
        let name = e.name.as_bytes();
        let name_len = name.len();

        let rec = match name_len.checked_add(8 + 1 + 2) {
            Some(r) => r,
            None => break,
        };
        match pos.checked_add(rec) {
            Some(end) if end <= out.len() => {}

            _ => break,
        }

        out[pos..pos + 8].copy_from_slice(&e.ino.to_le_bytes());
        pos += 8;

        out[pos] = kind_to_u8(e.kind);
        pos += 1;

        let nl = core::cmp::min(name_len, u16::MAX as usize) as u16;
        out[pos..pos + 2].copy_from_slice(&nl.to_le_bytes());
        pos += 2;

        out[pos..pos + name_len].copy_from_slice(name);
        pos += name_len;
    }

    pos as isize
}

fn sys_uptime_ms(_args: &[usize]) -> isize {
    let ms = crate::arch::xtensa::timer::uptime_ms();

    core::cmp::min(ms, isize::MAX as u64) as isize
}

fn sys_sbrk(_args: &[usize]) -> isize {
    let free = crate::mm::stats().free;
    core::cmp::min(free, isize::MAX as usize) as isize
}

fn sys_yield(_args: &[usize]) -> isize {
    crate::scheduler::set_need_resched();
    0
}

#[repr(C)]
struct Sigaction {
    sa_handler: usize,
    sa_flags: u32,
    sa_restorer: usize,
}

fn sys_sigaction(args: &[usize]) -> isize {
    let sig = arg(args, 0) as i32;
    let act_ptr = arg(args, 1) as *const Sigaction;

    if sig <= 0 || sig >= 32 || sig == 9 {
        return KError::InvalidArgument.as_errno();
    }
    if let Err(e) = validate_user(act_ptr as usize, core::mem::size_of::<Sigaction>()) {
        return e.as_errno();
    }

    if act_ptr.is_null() {
        return KError::Fault.as_errno();
    }

    let act = unsafe { &*act_ptr };

    let current_tid = crate::scheduler::current();
    let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();
    let pid = match pt.pid_of_tid(current_tid) {
        Some(p) => p,
        None => return KError::NotFound.as_errno(),
    };
    match pt.table.get_mut(&pid) {
        Some(proc) => {
            proc.signal_handlers[sig as usize] = act.sa_handler;
            proc.signal_restorers[sig as usize] = act.sa_restorer;
            0
        }
        None => KError::NotFound.as_errno(),
    }
}

fn sys_kill(args: &[usize]) -> isize {
    let pid = arg(args, 0) as u32;
    let sig = arg(args, 1) as i32;

    if sig <= 0 || sig >= 32 {
        return KError::InvalidArgument.as_errno();
    }

    let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();
    if let Some(proc) = pt.table.get_mut(&pid) {
        proc.pending_signals |= 1 << sig;

        crate::scheduler::unblock_task(proc.main_task);
        crate::scheduler::set_need_resched();

        return 0;
    }

    KError::NotFound.as_errno()
}

fn sys_sigreturn(_args: &[usize], frame: *mut esp_hal::xtensa_lx_rt::exception::Context) -> isize {
    let current_tid = crate::scheduler::current();
    let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();
    let pid = pt.pid_of_tid(current_tid);
    if let Some(proc) = pid.and_then(|p| pt.table.get_mut(&p)) {
        {
            if let Some(saved) = proc.saved_signal_context.take() {
                if !frame.is_null() {
                    unsafe {
                        *frame = saved;
                    }
                }
            }
            return 0;
        }
    }
    KError::NotFound.as_errno()
}

#[repr(C)]
struct sockaddr_in {
    sin_family: u16,
    sin_port: u16,
    sin_addr: u32,
    sin_zero: [u8; 8],
}

fn sys_socket(args: &[usize]) -> isize {
    let domain = arg(args, 0) as i32;
    let ty = arg(args, 1) as i32;
    let proto = arg(args, 2) as i32;

    let is_icmp = ty == 3 || proto == 1;
    let is_udp = ty == 2 && !is_icmp;

    if domain != 2 || (ty != 1 && ty != 2 && ty != 3 && proto != 1) {
        return KError::NotSupported.as_errno();
    }

    let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
    let sockets = match guard.as_mut() {
        Some(s) => s,
        None => return KError::IoError.as_errno(),
    };

    let handle = if is_icmp {
        let rx_meta = alloc::vec![smoltcp::socket::icmp::PacketMetadata::EMPTY; 8];
        let rx_data = alloc::vec![0; 2048];
        let tx_meta = alloc::vec![smoltcp::socket::icmp::PacketMetadata::EMPTY; 8];
        let tx_data = alloc::vec![0; 2048];
        let rx_buf = smoltcp::socket::icmp::PacketBuffer::new(rx_meta, rx_data);
        let tx_buf = smoltcp::socket::icmp::PacketBuffer::new(tx_meta, tx_data);
        let mut socket = smoltcp::socket::icmp::Socket::new(rx_buf, tx_buf);
        let _ = socket.bind(smoltcp::socket::icmp::Endpoint::Ident(0x1234));
        sockets.add(socket)
    } else if is_udp {
        let rx_meta = alloc::vec![smoltcp::socket::udp::PacketMetadata::EMPTY; 8];
        let rx_data = alloc::vec![0; 2048];
        let tx_meta = alloc::vec![smoltcp::socket::udp::PacketMetadata::EMPTY; 8];
        let tx_data = alloc::vec![0; 2048];
        let rx_buf = smoltcp::socket::udp::PacketBuffer::new(rx_meta, rx_data);
        let tx_buf = smoltcp::socket::udp::PacketBuffer::new(tx_meta, tx_data);
        let socket = smoltcp::socket::udp::Socket::new(rx_buf, tx_buf);
        sockets.add(socket)
    } else {
        let rx_buf = smoltcp::socket::tcp::SocketBuffer::new(alloc::vec![0; 4096]);
        let tx_buf = smoltcp::socket::tcp::SocketBuffer::new(alloc::vec![0; 4096]);
        let socket = smoltcp::socket::tcp::Socket::new(rx_buf, tx_buf);
        sockets.add(socket)
    };
    drop(guard);

    let socket_inode = Arc::new(crate::vfs::socket::SocketInode {
        handle: crate::arch::xtensa::sync::Mutex::new(handle),
        is_udp,
        is_icmp,
        remote_endpoint: crate::arch::xtensa::sync::Mutex::new(None),
        local_port: crate::arch::xtensa::sync::Mutex::new(None),
        recv_timeout_ms: core::sync::atomic::AtomicU32::new(0),
        non_blocking: core::sync::atomic::AtomicBool::new(false),
    });

    let open_file = match crate::vfs::OpenFile::new(socket_inode, crate::vfs::OpenFlags::RDWR) {
        Ok(f) => f,
        Err(e) => {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                sockets.remove(handle);
            }
            return e.as_errno();
        }
    };

    match crate::vfs::insert_open_file(open_file) {
        Ok(fd) => fd as isize,
        Err(e) => {
            let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
            if let Some(sockets) = guard.as_mut() {
                sockets.remove(handle);
            }
            e.as_errno()
        }
    }
}

fn sys_connect(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let addr_ptr = arg(args, 1) as *const sockaddr_in;
    let addr_len = arg(args, 2);

    if addr_ptr.is_null() || addr_len < core::mem::size_of::<sockaddr_in>() {
        return KError::InvalidArgument.as_errno();
    }
    if let Err(e) = validate_user(addr_ptr as usize, core::mem::size_of::<sockaddr_in>()) {
        return e.as_errno();
    }

    let addr = unsafe { &*addr_ptr };

    let inode = match crate::vfs::get_inode(fd) {
        Ok(inod) => inod,
        Err(e) => return e.as_errno(),
    };

    let handle = match inode.as_socket() {
        Some(h) => h,
        None => return KError::InvalidArgument.as_errno(),
    };

    let port = u16::from_be(addr.sin_port);
    let ip_bytes = addr.sin_addr.to_ne_bytes();
    let remote_addr =
        smoltcp::wire::IpAddress::Ipv4(smoltcp::wire::Ipv4Address::from_octets(ip_bytes));
    let remote_endpoint = smoltcp::wire::IpEndpoint::new(remote_addr, port);

    if inode.is_udp_socket() || inode.is_icmp_socket() {
        let _ = inode.set_socket_remote_endpoint(remote_endpoint);
        return 0;
    }

    let cmd = crate::drivers::wifi::NetCmd::Connect {
        handle,
        ip: ip_bytes,
        port,
    };
    crate::drivers::wifi::NET_CMD_QUEUE.lock().push(cmd);

    let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
    if let Some(sockets) = guard.as_mut() {
        let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
        match sock.state() {
            smoltcp::socket::tcp::State::Established => 0,
            smoltcp::socket::tcp::State::Closed => {
                if sock.remote_endpoint().is_some() {
                    KError::IoError.as_errno()
                } else {
                    KError::WouldBlock.as_errno()
                }
            }
            smoltcp::socket::tcp::State::SynSent => KError::WouldBlock.as_errno(),
            _ => KError::IoError.as_errno(),
        }
    } else {
        KError::IoError.as_errno()
    }
}

fn sys_bind(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let addr_ptr = arg(args, 1) as *const sockaddr_in;
    let addr_len = arg(args, 2);

    if addr_ptr.is_null() || addr_len < core::mem::size_of::<sockaddr_in>() {
        return KError::InvalidArgument.as_errno();
    }
    if let Err(e) = validate_user(addr_ptr as usize, core::mem::size_of::<sockaddr_in>()) {
        return e.as_errno();
    }
    let addr = unsafe { &*addr_ptr };
    let port = u16::from_be(addr.sin_port);

    match crate::vfs::get_inode(fd) {
        Ok(inode) => match inode.bind(port) {
            Ok(()) => 0,
            Err(e) => e.as_errno(),
        },
        Err(e) => e.as_errno(),
    }
}

fn sys_listen(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let backlog = arg(args, 1) as i32;

    match crate::vfs::get_inode(fd) {
        Ok(inode) => match inode.listen(backlog) {
            Ok(()) => 0,
            Err(e) => e.as_errno(),
        },
        Err(e) => e.as_errno(),
    }
}

fn sys_accept(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;

    let listen_inode = match crate::vfs::get_inode(fd) {
        Ok(inode) => inode,
        Err(e) => return e.as_errno(),
    };

    match listen_inode.accept() {
        Ok(accepted_inode) => {
            let open_file =
                match crate::vfs::OpenFile::new(accepted_inode, crate::vfs::OpenFlags::RDWR) {
                    Ok(f) => f,
                    Err(e) => return e.as_errno(),
                };
            match crate::vfs::insert_open_file(open_file) {
                Ok(new_fd) => new_fd as isize,
                Err(e) => e.as_errno(),
            }
        }
        Err(e) => e.as_errno(),
    }
}

#[repr(C)]
struct timeval {
    tv_sec: i32,
    tv_usec: i32,
}

fn sys_gettimeofday(args: &[usize]) -> isize {
    let tv_ptr = arg(args, 0) as *mut timeval;
    if let Err(e) = validate_user(tv_ptr as usize, core::mem::size_of::<timeval>()) {
        return e.as_errno();
    }

    let uptime_us = esp_hal::time::now().duration_since_epoch().to_micros();
    let offset_us = *crate::arch::xtensa::timer::SYSTEM_TIME_OFFSET_US.lock();
    let total_us = uptime_us.saturating_add(offset_us);

    let sec = (total_us / 1_000_000) as i32;
    let usec = (total_us % 1_000_000) as i32;

    unsafe {
        (*tv_ptr).tv_sec = sec;
        (*tv_ptr).tv_usec = usec;
    }
    0
}

fn sys_settimeofday(args: &[usize]) -> isize {
    let tv_ptr = arg(args, 0) as *const timeval;
    if let Err(e) = validate_user(tv_ptr as usize, core::mem::size_of::<timeval>()) {
        return e.as_errno();
    }

    let tv = unsafe { &*tv_ptr };
    let new_total_us = (tv.tv_sec as u64)
        .saturating_mul(1_000_000)
        .saturating_add(tv.tv_usec as u64);

    let uptime_us = esp_hal::time::now().duration_since_epoch().to_micros();
    let new_offset_us = new_total_us.saturating_sub(uptime_us);

    *crate::arch::xtensa::timer::SYSTEM_TIME_OFFSET_US.lock() = new_offset_us;
    0
}

fn sys_ota_state(args: &[usize]) -> isize {
    let op = arg(args, 0);
    if op == 0 {
        match crate::ota::get_state() {
            Ok(state) => state.as_raw() as isize,
            Err(e) => e.as_errno(),
        }
    } else if op == 1 {
        let state_raw = arg(args, 1) as u32;
        let state = crate::ota::OtaImgState::from_raw(state_raw);
        match crate::ota::set_state(state) {
            Ok(()) => {
                if state_raw == crate::ota::OtaImgState::Invalid.as_raw()
                    || state_raw == crate::ota::OtaImgState::Aborted.as_raw()
                {
                    crate::println!("[kernel] Partition marked as invalid/aborted. Rebooting...");
                    esp_hal::reset::software_reset();
                }
                0
            }
            Err(e) => e.as_errno(),
        }
    } else {
        KError::InvalidArgument.as_errno()
    }
}
