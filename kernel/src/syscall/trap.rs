//! Trap real de la instrucción `syscall` (EXCCAUSE=1) — sólo con `syscall-trap`.
//!
//! El vector de excepción de xtensa-lx-rt vuelca los registros en un `Context`
//! y llama a `__exception`. Ese símbolo es débil (por defecto reenvía a
//! `__user_exception`, que provee esp-backtrace). Aquí lo **sobreescribimos**:
//! atendemos EXCCAUSE=1 despachando el syscall, y delegamos cualquier otra causa
//! en `__user_exception` para no perder el backtrace/panic de esp-backtrace.
//!
//! Sin separación de privilegios todavía, esto ejercita el mecanismo de trap
//! (no aísla). La validación de punteros de usuario en `handler.rs` sigue siendo
//! best-effort hasta que existan anillos (Fase 8 / userland).

use esp_hal::xtensa_lx_rt::exception::{Context, ExceptionCause};

/// Longitud de la instrucción `syscall` en Xtensa (24 bits = 3 bytes).
const SYSCALL_INSN_LEN: u32 = 3;

extern "C" {
    /// Handler de excepción de esp-backtrace (feature `exception-handler`).
    fn __user_exception(cause: ExceptionCause, save_frame: &mut Context);
}

/// Sobreescribe el `__exception` débil de xtensa-lx-rt. La CPU llega aquí con los
/// registros ya volcados en `save_frame`.
///
/// # Safety
/// La firma debe coincidir exactamente con la que invoca el vector de excepción
/// (`unsafe extern "C" fn(ExceptionCause, &mut Context)`).
#[no_mangle]
#[link_section = ".rwtext"]
unsafe extern "C" fn __exception(cause: ExceptionCause, save_frame: &mut Context) {
    if cause == ExceptionCause::Syscall {
        let num = save_frame.A2 as usize;
        let args = [
            save_frame.A3 as usize,
            save_frame.A4 as usize,
            save_frame.A5 as usize,
            save_frame.A6 as usize,
            save_frame.A7 as usize,
            save_frame.A8 as usize,
        ];
        // DIAGNÓSTICO temporal: traza los primeros syscalls para localizar dónde
        // llega el userland.
        {
            use core::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let c = N.fetch_add(1, Ordering::Relaxed);
            if c < 12 {
                esp_println::println!(
                    "[trap] #{} syscall num={} a3={:#x} a4={:#x} pc={:#x}",
                    c, num, save_frame.A3, save_frame.A4, save_frame.PC
                );
            }
        }
        let ret = crate::syscall::dispatch(num, &args, save_frame as *mut Context);
        save_frame.A2 = ret as u32;
        // Avanzar EPC más allá de `syscall` para no re-ejecutarla al volver.
        save_frame.PC = save_frame.PC.wrapping_add(SYSCALL_INSN_LEN);

        let mut next_sp: Option<u32> = None;
        if crate::scheduler::need_resched() {
            next_sp = crate::scheduler::preempt_switch(save_frame);
        }

        if let Some(sp) = next_sp {
            let next_frame = &mut *(sp as *mut Context);
            let _ = crate::scheduler::process::check_signals(next_frame);
            core::arch::asm!(
                "mov a5, {0}",
                "movi a4, 1",
                "rsr.windowbase a6",
                "ssl a6",
                "sll a4, a4",
                "wsr.windowstart a4",
                "rsync",
                in(reg) sp,
                out("a6") _,
                out("a4") _
            );
        } else {
            let _ = crate::scheduler::process::check_signals(save_frame);
        }
        return;
    }
    // Cualquier otra causa: diagnóstico de esp-backtrace intacto.
    __user_exception(cause, save_frame);
}
