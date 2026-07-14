#![allow(dead_code)]

pub mod handler;
pub mod table;

#[cfg(feature = "syscall-trap")]
mod trap;

pub use handler::dispatch;
pub use table::Syscall;

pub const MAX_ARGS: usize = 6;

#[cfg(not(feature = "syscall-trap"))]
#[inline]
pub fn invoke(num: usize, args: [usize; MAX_ARGS]) -> isize {
    dispatch(num, &args, core::ptr::null_mut())
}

#[cfg(feature = "syscall-trap")]
#[inline(never)]
pub fn invoke(num: usize, args: [usize; MAX_ARGS]) -> isize {
    let ret: isize;

    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("a2") num => ret,
            in("a3") args[0],
            in("a4") args[1],
            in("a5") args[2],
            in("a6") args[3],
            in("a7") args[4],
            in("a8") args[5],
            options(nostack),
        );
    }
    ret
}
