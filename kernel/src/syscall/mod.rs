#![allow(dead_code)]

pub mod handler;
pub mod table;

#[cfg(feature = "syscall-trap")]
mod trap;

pub use handler::dispatch;
pub use table::Syscall;

/// Número de argumentos que transporta la ABI de syscall.
pub const MAX_ARGS: usize = 6;

/// Invoca un syscall por su número con hasta 6 argumentos.
///
/// Sin la feature `syscall-trap` (por defecto) es una **puerta software** directa
/// al despachador: ejercita la misma ABI (marshalling de argumentos, dispatch y
/// errno) sin trap de CPU. Como todavía no hay userland/anillos, un trap no
/// añadiría aislamiento, así que desde el lado del kernel es equivalente.
///
/// Con `--features syscall-trap` emite la instrucción `syscall` real y la CPU
/// entra por el vector de excepción (EXCCAUSE=1), que despacha en [`trap`].
#[cfg(not(feature = "syscall-trap"))]
#[inline]
pub fn invoke(num: usize, args: [usize; MAX_ARGS]) -> isize {
    dispatch(num, &args)
}

#[cfg(feature = "syscall-trap")]
#[inline(never)]
pub fn invoke(num: usize, args: [usize; MAX_ARGS]) -> isize {
    let ret: isize;
    // Convención: a2=número, a3..a8=argumentos, retorno en a2.
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
