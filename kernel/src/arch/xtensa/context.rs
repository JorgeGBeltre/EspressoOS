#![allow(dead_code)]

use core::arch::asm;
use esp_hal::xtensa_lx_rt::exception::Context as ExceptionContext;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    /// Estado completo de CPU de la tarea. ES el frame que el vector de
    /// excepción/interrupción restaura; conmutar de tarea = copiar este frame
    /// dentro de `*save_frame` (el mecanismo de esp-wifi/esp-hal).
    pub frame: ExceptionContext,
}

const STACK_ALIGN_MASK: usize = 0xF;

// Constantes PS de Xtensa
const PS_UM: u32 = 1 << 5;       // User Mode
const PS_WOE: u32 = 1 << 18;     // Window Overflow Enable
const PS_CALLINC1: u32 = 1 << 16; // Call Increment 1

#[inline(never)]
pub fn init_task_stack(stack_top: *mut u8, entry: fn(usize), arg: usize, is_user: bool) -> Context {
    // Frame inicial POR VALOR (no carvado en la pila). Se copia a *save_frame en
    // la primera conmutación (o se inyecta vía resume_task en el arranque).
    let top = ((stack_top as usize) & !STACK_ALIGN_MASK) as u32;

    let mut frame = ExceptionContext::default();
    frame.PC = entry as usize as u32;
    frame.PS = if is_user {
        PS_UM | PS_WOE | PS_CALLINC1
    } else {
        PS_WOE | PS_CALLINC1 // modo privilegiado (kernel)
    };
    frame.A0 = 0; // dir. de retorno (si la tarea retorna, faultea; lo maneja exit())
    frame.A1 = top; // stack pointer real de la tarea
    frame.A6 = arg as u32; // convenio call4: con PS.CALLINC=1, `entry` rota A6 -> A2

    Context { frame }
}

#[inline(always)]
pub unsafe fn resume_task(sp: u32) -> ! {
    asm!(
        // Establecer el stack pointer al frame del ExceptionContext
        "mov a1, {0}",
        
        // Resetear windowstart y windowbase para asegurar que el registro de ventanas empiece limpio
        "movi a4, 1",
        "rsr.windowbase a5",
        "ssl a5",
        "sll a4, a4",
        "wsr.windowstart a4",
        "rsync",
        
        // Llamar a restore_context de xtensa-lx-rt usando callx0 para largo alcance
        "movi a0, restore_context",
        "callx0 a0",
        
        // Restaurar PS, EPC1 y registers especiales
        "l32i a0, a1, 4",  // XT_STK_PS = 4
        "wsr a0, PS",
        "l32i a0, a1, 0",  // XT_STK_PC = 0
        "wsr a0, EPC1",
        "rsync",
        
        // Restaurar A0 y A1 (el stack pointer real)
        "l32i a0, a1, 8",  // XT_STK_A0 = 8
        "l32i a1, a1, 12", // XT_STK_A1 = 12
        "rsync",
        
        // Retornar de la excepción
        "rfe",
        in(reg) sp,
        options(noreturn)
    );
}
