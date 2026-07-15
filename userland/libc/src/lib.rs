#![no_std]
#![feature(naked_functions, asm_experimental_arch)]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::fmt::{Write, Result};





extern "Rust" {
    fn main(argc: i32, argv: *const *const u8) -> i32;
}

/// Entry point. The kernel hands one usize in a2: a pointer to the argv blob it
/// wrote into the top of this program's data slot, laid out as
///
///     [argc: u32][argv[0]]..[argv[argc-1]][NULL][strings...]
///
/// main takes two parameters and the kernel only passes one, so `_start` unpacks:
/// argc is the first word and argv is the blob plus four.
///
/// The `entry` below is load-bearing and easy to leave out.
///
/// On Xtensa the register window is not rotated by the call -- it is rotated by the
/// `entry` instruction in the callee's prologue, by PS.CALLINC*4. A #[naked]
/// function has no prologue, so without this it runs in the CALLER's window and a2
/// holds whatever the caller had there, not the argument. The kernel invokes this
/// through a callx8, which by the ABI leaves the argument in a10; `entry` rotates
/// by eight and brings it to a2. Reading a10 directly would work today and break
/// the day the compiler picks call4 or call12.
///
/// Then, since call4 rotates the callee's window by four, the caller's a6..a9 land
/// in the callee's a2..a5 -- so argc in a6 and argv in a7 is what puts them in
/// main's a2 and a3.
#[no_mangle]
#[unsafe(naked)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "entry a1, 32",
        "l32i  a6, a2, 0",
        "addi  a7, a2, 4",
        "call4 main",
        "mov a2, a6",
        "call4 exit",
        "loop:",
        "j loop"
    );
}

/// The `i`th argument as a string.
///
/// # Safety
/// `argv` must be the pointer main was given, and `i` must be less than its argc.
pub unsafe fn arg(argv: *const *const u8, i: i32) -> &'static str {
    let p = *argv.offset(i as isize);
    if p.is_null() {
        return "";
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len))
}





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


pub fn pipe(fds: &mut [i32; 2]) -> isize {
    unsafe { syscall(26, fds.as_mut_ptr() as usize, 0, 0, 0, 0, 0) }
}

pub fn dup2(oldfd: i32, newfd: i32) -> isize {
    unsafe { syscall(27, oldfd as usize, newfd as usize, 0, 0, 0, 0) }
}

pub fn sbrk(size: usize) -> isize {
    unsafe { syscall(13, size, 0, 0, 0, 0, 0) }
}

pub fn yield_now() {
    unsafe { syscall(14, 0, 0, 0, 0, 0, 0); }
}

#[repr(C)]
pub struct timeval {
    pub tv_sec: i32,
    pub tv_usec: i32,
}

pub fn gettimeofday(tv: &mut timeval) -> isize {
    unsafe { syscall(23, tv as *mut timeval as usize, 0, 0, 0, 0, 0) }
}

pub fn settimeofday(tv: &timeval) -> isize {
    unsafe { syscall(24, tv as *const timeval as usize, 0, 0, 0, 0, 0) }
}

#[repr(C)]
pub struct sockaddr_in {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: u32,
    pub sin_zero: [u8; 8],
}

pub fn socket(domain: i32, ty: i32, protocol: i32) -> isize {
    unsafe { syscall(18, domain as usize, ty as usize, protocol as usize, 0, 0, 0) }
}

pub fn bind(fd: i32, addr: &sockaddr_in) -> isize {
    unsafe { syscall(19, fd as usize, addr as *const sockaddr_in as usize, core::mem::size_of::<sockaddr_in>(), 0, 0, 0) }
}

pub fn listen(fd: i32, backlog: i32) -> isize {
    unsafe { syscall(20, fd as usize, backlog as usize, 0, 0, 0, 0) }
}

pub fn accept(fd: i32, addr: &mut sockaddr_in) -> isize {
    let mut len = core::mem::size_of::<sockaddr_in>();
    unsafe { syscall(21, fd as usize, addr as *mut sockaddr_in as usize, &mut len as *mut usize as usize, 0, 0, 0) }
}

pub fn connect(fd: i32, addr: &sockaddr_in) -> isize {
    unsafe { syscall(22, fd as usize, addr as *const sockaddr_in as usize, core::mem::size_of::<sockaddr_in>(), 0, 0, 0) }
}

pub fn send(fd: i32, buf: &[u8]) -> isize {
    write(fd, buf)
}

pub fn recv(fd: i32, buf: &mut [u8]) -> isize {
    read(fd, buf)
}

pub fn ota_state(op: usize, state: usize) -> isize {
    unsafe { syscall(25, op, state, 0, 0, 0, 0) }
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





#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    exit(-1);
}
