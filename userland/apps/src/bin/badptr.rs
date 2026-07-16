#![no_std]
#![no_main]

use libc::{println, syscall};

const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_WAIT: usize = 7;

const EFAULT: isize = -14;

/// A garbage address. Nothing is mapped here.
const GARBAGE: usize = 0xDEAD_BEEF;

/// A REAL kernel static, taken from a `LoadProhibited` backtrace this kernel
/// printed earlier: `0x3fc9620c - kernel::session::NEXT_ID`. Internal SRAM, nowhere
/// near this process. Before validate_user existed, `wait(0x3fc9620c)` would have
/// stored an exit code straight over the session id counter.
const KERNEL_STATIC: usize = 0x3fc9_620c;

fn check(name: &str, got: isize, want: isize) -> bool {
    if got == want {
        println!("  ok    {} -> {}", name, got);
        true
    } else {
        println!("  FAIL  {} -> {} (expected {})", name, got, want);
        false
    }
}

/// Proves the syscall layer rejects pointers this process does not own.
///
/// The other half of the test: `/bin/echo | /bin/cat` shows that legitimate
/// pointers pass. Without this one, a validator that returned Ok for everything
/// would look identical.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut ok = true;
    println!("badptr: pointers this process does not own");

    // sys_wait is the one that started it: it wrote the exit code wherever it was
    // pointed, checking only for null.
    ok &= check(
        "wait(0xDEADBEEF)",
        unsafe { syscall(SYS_WAIT, GARBAGE, 0, 0, 0, 0, 0) },
        EFAULT,
    );
    ok &= check(
        "wait(&kernel::session::NEXT_ID)",
        unsafe { syscall(SYS_WAIT, KERNEL_STATIC, 0, 0, 0, 0, 0) },
        EFAULT,
    );

    // These go through user_slice / user_slice_mut.
    ok &= check(
        "write(1, 0xDEADBEEF, 8)",
        unsafe { syscall(SYS_WRITE, 1, GARBAGE, 8, 0, 0, 0) },
        EFAULT,
    );
    ok &= check(
        "read(0, &kernel_static, 8)",
        unsafe { syscall(SYS_READ, 0, KERNEL_STATIC, 8, 0, 0, 0) },
        EFAULT,
    );

    // A control, so a validator that rejected everything would not pass either: a
    // buffer on this process's own stack has to work.
    println!("badptr: pointers this process does own");
    let buf = *b"control\n";
    let n = unsafe { syscall(SYS_WRITE, 1, buf.as_ptr() as usize, buf.len(), 0, 0, 0) };
    ok &= check("write(1, &stack_buf, 8)", n, 8);

    if ok {
        println!("badptr: PASS");
        0
    } else {
        println!("badptr: FAIL");
        1
    }
}
