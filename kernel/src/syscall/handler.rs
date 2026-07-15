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

unsafe fn user_slice<'a>(ptr: usize, len: usize) -> KResult<&'a [u8]> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr == 0 {
        return Err(KError::Fault);
    }

    Ok(core::slice::from_raw_parts(ptr as *const u8, len))
}

unsafe fn user_slice_mut<'a>(ptr: usize, len: usize) -> KResult<&'a mut [u8]> {
    if len == 0 {
        return Ok(&mut []);
    }
    if ptr == 0 {
        return Err(KError::Fault);
    }

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
    let mut parent_to_wake = None;

    {
        let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();
        let mut current_pid = None;
        for (&pid, proc) in &mut pt.table {
            if proc.main_task == current_tid {
                proc.state = crate::scheduler::process::ProcessState::Zombie;
                proc.exit_code = code;
                current_pid = Some(pid);
                break;
            }
        }

        if let Some(pid) = current_pid {
            if let Some(proc) = pt.table.get(&pid) {
                if let Some(parent_pid) = proc.parent_pid {
                    if let Some(parent_proc) = pt.table.get(&parent_pid) {
                        parent_to_wake = Some(parent_proc.main_task);
                    }
                }
            }
        }
    }

    if let Some(parent_tid) = parent_to_wake {
        crate::scheduler::unblock_task(parent_tid);
    }

    crate::scheduler::mark_zombie(code);
    crate::scheduler::set_need_resched();
    0
}

fn sys_spawn(args: &[usize]) -> isize {
    let name = match unsafe { user_str(arg(args, 0), arg(args, 1)) } {
        Ok(s) => s,
        Err(e) => return e.as_errno(),
    };
    let entry_raw = arg(args, 2);

    if entry_raw == 0 {
        match crate::fs::elf::load_elf(name) {
            Ok(loaded) => {
                let entry: fn(usize) = unsafe { core::mem::transmute(loaded.entry as usize) };
                match crate::scheduler::spawn_blocked(
                    name,
                    entry,
                    0,
                    layout::DEFAULT_STACK_SIZE,
                    10,
                    true,
                ) {
                    Ok(tid) => {
                        // The slot goes on the process: reaping it is what returns
                        // the slot to the pool.
                        let pid = crate::scheduler::process::register_process(
                            name,
                            tid,
                            true,
                            Some(loaded.slot),
                        );
                        crate::scheduler::unblock_task(tid);
                        pid as isize
                    }
                    Err(e) => {
                        // Nothing owns the image yet, so the slot is ours to drop.
                        crate::mm::psram_exec::slot_free(loaded.slot);
                        e.as_errno()
                    }
                }
            }
            Err(e) => e.as_errno(),
        }
    } else {
        let entry: fn(usize) = unsafe { core::mem::transmute::<usize, fn(usize)>(entry_raw) };
        let entry_arg = arg(args, 3);
        let mut stack_size = arg(args, 4);
        if stack_size == 0 {
            stack_size = layout::DEFAULT_STACK_SIZE;
        }
        let priority = arg(args, 5) as u8;

        match crate::scheduler::spawn_blocked(name, entry, entry_arg, stack_size, priority, false) {
            Ok(tid) => {
                // A bare entry point, not an image: no slot to own.
                let pid = crate::scheduler::process::register_process(name, tid, false, None);
                crate::scheduler::unblock_task(tid);
                pid as isize
            }
            Err(e) => e.as_errno(),
        }
    }
}

fn sys_wait(args: &[usize]) -> isize {
    let status_ptr = arg(args, 0) as *mut i32;
    let current_tid = crate::scheduler::current();

    let mut current_pid = None;
    {
        let pt = crate::scheduler::process::PROCESS_TABLE.lock();
        for (&pid, proc) in &pt.table {
            if proc.main_task == current_tid {
                current_pid = Some(pid);
                break;
            }
        }
    }
    let current_pid = match current_pid {
        Some(pid) => pid,
        None => return KError::NotFound.as_errno(),
    };

    let mut reaped: Option<(u32, i32, Option<crate::mm::psram_exec::SlotIndex>)> = None;
    {
        let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();

        let (children_empty, zombie_pid) = {
            let current_proc = match pt.table.get(&current_pid) {
                Some(p) => p,
                None => return KError::NotFound.as_errno(),
            };
            if current_proc.children.is_empty() {
                (true, None)
            } else {
                let mut z = None;
                for &child_pid in &current_proc.children {
                    if let Some(child_proc) = pt.table.get(&child_pid) {
                        if child_proc.state == crate::scheduler::process::ProcessState::Zombie {
                            z = Some(child_pid);
                            break;
                        }
                    }
                }
                (false, z)
            }
        };

        if children_empty {
            return KError::NotFound.as_errno();
        }

        if let Some(child_pid) = zombie_pid {
            let child_proc = pt.table.remove(&child_pid).unwrap();
            if let Some(current_proc_mut) = pt.table.get_mut(&current_pid) {
                current_proc_mut.children.retain(|&x| x != child_pid);
            }
            reaped = Some((child_pid, child_proc.exit_code, child_proc.slot));
        }
    }

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

    if act_ptr.is_null() {
        return KError::Fault.as_errno();
    }

    let act = unsafe { &*act_ptr };

    let current_tid = crate::scheduler::current();
    let mut pt = crate::scheduler::process::PROCESS_TABLE.lock();
    for proc in pt.table.values_mut() {
        if proc.main_task == current_tid {
            proc.signal_handlers[sig as usize] = act.sa_handler;
            proc.signal_restorers[sig as usize] = act.sa_restorer;
            return 0;
        }
    }

    KError::NotFound.as_errno()
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
    for proc in pt.table.values_mut() {
        if proc.main_task == current_tid {
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
    let _proto = arg(args, 2) as i32;

    if domain != 2 || (ty != 1 && ty != 2) {
        return KError::NotSupported.as_errno();
    }

    let is_udp = ty == 2;

    let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
    let sockets = match guard.as_mut() {
        Some(s) => s,
        None => return KError::IoError.as_errno(),
    };

    let handle = if is_udp {
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
        remote_endpoint: crate::arch::xtensa::sync::Mutex::new(None),
        local_port: crate::arch::xtensa::sync::Mutex::new(None),
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

    if inode.is_udp_socket() {
        let _ = inode.set_socket_remote_endpoint(remote_endpoint);
        return 0;
    }

    let cmd = crate::drivers::wifi::NetCmd::Connect {
        handle,
        ip: ip_bytes,
        port,
    };
    crate::drivers::wifi::NET_CMD_QUEUE.lock().push(cmd);

    loop {
        let mut guard = crate::drivers::wifi::NET_SOCKETS.lock();
        if let Some(sockets) = guard.as_mut() {
            let sock = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
            match sock.state() {
                smoltcp::socket::tcp::State::Established => return 0,
                smoltcp::socket::tcp::State::Closed => {
                    if sock.remote_endpoint().is_some() {
                        return KError::IoError.as_errno();
                    }
                }
                smoltcp::socket::tcp::State::SynSent => {}
                _ => return KError::IoError.as_errno(),
            }
        }
        drop(guard);
        crate::scheduler::yield_now();
    }
}

fn sys_bind(args: &[usize]) -> isize {
    let fd = arg(args, 0) as Fd;
    let addr_ptr = arg(args, 1) as *const sockaddr_in;
    let addr_len = arg(args, 2);

    if addr_ptr.is_null() || addr_len < core::mem::size_of::<sockaddr_in>() {
        return KError::InvalidArgument.as_errno();
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
    if tv_ptr.is_null() {
        return KError::InvalidArgument.as_errno();
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
    if tv_ptr.is_null() {
        return KError::InvalidArgument.as_errno();
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
