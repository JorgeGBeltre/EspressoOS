#![allow(dead_code)]

use core::arch::asm;
use esp_hal::xtensa_lx_rt::exception::Context as ExceptionContext;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Context {
    pub sp: u32,
}

const STACK_ALIGN_MASK: usize = 0xF;

// Constantes PS de Xtensa
const PS_UM: u32 = 1 << 5;       // User Mode
const PS_WOE: u32 = 1 << 18;     // Window Overflow Enable
const PS_CALLINC1: u32 = 1 << 16; // Call Increment 1

#[inline(never)]
pub fn init_task_stack(stack_top: *mut u8, entry: fn(usize), arg: usize, is_user: bool) -> Context {
    let top = (stack_top as usize) & !STACK_ALIGN_MASK;
    
    // Reservar espacio para la estructura ExceptionContext completa
    let frame_base = top - core::mem::size_of::<ExceptionContext>();
    let frame_base = frame_base & !STACK_ALIGN_MASK;
    
    let frame_ptr = frame_base as *mut ExceptionContext;
    
    unsafe {
        // Inicializar a cero
        core::ptr::write_bytes(frame_ptr, 0, 1);
        
        let frame = &mut *frame_ptr;
        frame.PC = entry as usize as u32;
        if is_user {
            frame.PS = PS_UM | PS_WOE | PS_CALLINC1;
        } else {
            frame.PS = PS_WOE | PS_CALLINC1; // Privileged mode
        }
        frame.A0 = 0; // Dirección de retorno (llamará a la salida de la tarea si retorna)
        frame.A1 = top as u32; // Stack pointer de la tarea (cuando se retire el ExceptionContext)
        frame.A6 = arg as u32; // Argumento a pasar a la tarea (convenio call4)
    }

    Context {
        sp: frame_base as u32,
    }
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
