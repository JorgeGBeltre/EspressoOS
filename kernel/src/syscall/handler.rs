#![allow(dead_code)]

use crate::prelude::*;
use crate::vfs::{Fd, InodeKind, OpenFlags, SeekFrom};
use super::table::Syscall;

pub fn dispatch(num: usize, args: &[usize]) -> isize {
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

fn sys_ioctl(_args: &[usize]) -> isize {
    KError::NotSupported.as_errno()
}

fn sys_exit(args: &[usize]) -> isize {
    let code = arg(args, 0) as i32;
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
        return KError::Fault.as_errno();
    }

    let entry: fn(usize) = unsafe { core::mem::transmute::<usize, fn(usize)>(entry_raw) };
    let entry_arg = arg(args, 3);
    let mut stack_size = arg(args, 4);
    if stack_size == 0 {
        stack_size = layout::DEFAULT_STACK_SIZE;
    }
    let priority = arg(args, 5) as u8;

    match crate::scheduler::spawn(name, entry, entry_arg, stack_size, priority) {
        Ok(tid) => tid as isize,
        Err(e) => e.as_errno(),
    }
}

fn sys_wait(_args: &[usize]) -> isize {
    KError::NotSupported.as_errno()
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
