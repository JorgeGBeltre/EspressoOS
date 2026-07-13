#![no_std]
#![feature(naked_functions, asm_experimental_arch)]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::fmt::{Write, Result};

// ===========================================================================
// Runtime Entry Point (crt0)
// ===========================================================================

extern "Rust" {
    fn main() -> i32;
}

#[no_mangle]
#[unsafe(naked)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "movi a1, 0", // a1 ya está apuntando al tope de la pila por el kernel
        "call4 main",
        "mov a2, a6", // Código de retorno de main en la ventana rotada
        "call4 exit",
        "loop:",
        "j loop"
    );
}

// ===========================================================================
// System Call Stubs
// ===========================================================================

#[inline(never)]
pub unsafe fn syscall(
    num: usize,
    arg0: usize,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> isize {
    let ret: isize;
    core::arch::asm!(
        "syscall",
        inlateout("a2") num => ret,
        in("a3") arg0,
        in("a4") arg1,
        in("a5") arg2,
        in("a6") arg3,
        in("a7") arg4,
        in("a8") arg5,
        options(nostack),
    );
    ret
}

pub fn read(fd: i32, buf: &mut [u8]) -> isize {
    unsafe { syscall(0, fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0, 0) }
}

pub fn write(fd: i32, buf: &[u8]) -> isize {
    unsafe { syscall(1, fd as usize, buf.as_ptr() as usize, buf.len(), 0, 0, 0) }
}

pub fn open(path: &str, flags: u32) -> isize {
    unsafe { syscall(2, path.as_ptr() as usize, path.len(), flags as usize, 0, 0, 0) }
}

pub fn close(fd: i32) -> isize {
    unsafe { syscall(3, fd as usize, 0, 0, 0, 0, 0) }
}

#[no_mangle]
pub fn exit(code: i32) -> ! {
    unsafe {
        syscall(5, code as usize, 0, 0, 0, 0, 0);
    }
    loop {}
}

pub fn spawn(path: &str, entry: usize, arg: usize, stack_size: usize, priority: usize) -> isize {
    unsafe {
        syscall(6, path.as_ptr() as usize, path.len(), entry, arg, stack_size, priority)
    }
}

pub fn wait(status: &mut i32) -> isize {
    unsafe { syscall(7, status as *mut i32 as usize, 0, 0, 0, 0, 0) }
}

pub fn seek(fd: i32, offset: isize, whence: i32) -> isize {
    unsafe { syscall(8, fd as usize, offset as usize, whence as usize, 0, 0, 0) }
}

pub fn mkdir(path: &str) -> isize {
    unsafe { syscall(9, path.as_ptr() as usize, path.len(), 0, 0, 0, 0) }
}

pub fn unlink(path: &str) -> isize {
    unsafe { syscall(10, path.as_ptr() as usize, path.len(), 0, 0, 0, 0) }
}

pub fn readdir(path: &str, buf: &mut [u8]) -> isize {
    unsafe { syscall(11, path.as_ptr() as usize, path.len(), buf.as_mut_ptr() as usize, buf.len(), 0, 0) }
}

pub fn uptime_ms() -> usize {
    unsafe { syscall(12, 0, 0, 0, 0, 0, 0) as usize }
}

pub fn sbrk(size: usize) -> isize {
    unsafe { syscall(13, size, 0, 0, 0, 0, 0) }
}

pub fn yield_now() {
    unsafe { syscall(14, 0, 0, 0, 0, 0, 0); }
}

pub fn signal(sig: i32, handler: usize, restorer: usize) -> isize {
    #[repr(C)]
    struct Sigaction {
        sa_handler: usize,
        sa_flags: u32,
        sa_restorer: usize,
    }
    let act = Sigaction {
        sa_handler: handler,
        sa_flags: 0,
        sa_restorer: restorer,
    };
    unsafe { syscall(15, sig as usize, &act as *const Sigaction as usize, 0, 0, 0, 0) }
}

pub fn kill(pid: u32, sig: i32) -> isize {
    unsafe { syscall(16, pid as usize, sig as usize, 0, 0, 0, 0) }
}

#[no_mangle]
pub extern "C" fn sigreturn_trampoline() {
    unsafe {
        syscall(17, 0, 0, 0, 0, 0, 0);
    }
}

// ===========================================================================
// Memory Allocator
// ===========================================================================

struct SimpleBumpAllocator {
    heap: UnsafeCell<[u8; 32768]>,
    next: UnsafeCell<usize>,
}

unsafe impl Sync for SimpleBumpAllocator {}

impl SimpleBumpAllocator {
    const fn new() -> Self {
        Self {
            heap: UnsafeCell::new([0; 32768]),
            next: UnsafeCell::new(0),
        }
    }
}

unsafe impl GlobalAlloc for SimpleBumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        let next = &mut *self.next.get();
        
        let aligned = (*next + align - 1) & !(align - 1);
        if aligned + size > 32768 {
            return core::ptr::null_mut();
        }
        
        *next = aligned + size;
        (self.heap.get() as *mut u8).add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: SimpleBumpAllocator = SimpleBumpAllocator::new();

// ===========================================================================
// Format/Print
// ===========================================================================

struct ConsoleWriter;

impl Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> Result {
        let _ = write(1, s.as_bytes());
        Ok(())
    }
}

pub fn _print(args: core::fmt::Arguments) {
    let mut w = ConsoleWriter;
    let _ = w.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::_print(format_args!($($arg)*));
        $crate::print!("\n");
    }};
}

// ===========================================================================
// Panic Handler
// ===========================================================================

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    exit(-1);
}
