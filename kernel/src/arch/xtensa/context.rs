//! Cambio de contexto en Xtensa LX7 (Fase 2) — la pieza MÁS delicada del kernel.
//!
//! El Xtensa LX7 del ESP32-S3 usa *registros con ventana* (windowed ABI): hay
//! 64 registros físicos de dirección (AR0..AR63) y en cada instante sólo se ven
//! 16 (`a0..a15`) situados por `WINDOWBASE`. `WINDOWSTART` es un mapa de bits que
//! marca qué grupos de 4 registros son el inicio de un marco de llamada vivo. Un
//! cambio de contexto correcto DEBE volcar (spill) todas las ventanas activas de
//! la tarea saliente a SU pila, para poder reconstruirlas por *window underflow*
//! cuando se reanude.
//!
//! ## Modelo de este archivo (documentado y justificado)
//!
//! `switch_to` distingue dos rutas para la tarea entrante (`next`):
//!
//! 1. **Reanudación** (`first_run == 0`): la tarea ya corrió y se suspendió
//!    DENTRO de `switch_to` (la llamó el planificador). Guardamos lo mínimo
//!    (a0 = dirección de retorno windowed, a1 = SP, PS), volcamos las ventanas y
//!    al restaurar hacemos `retw`: el hardware devuelve el control al llamador de
//!    `switch_to` de esa tarea (p. ej. `scheduler::schedule`), reconstruyendo sus
//!    a2..a15 desde la pila volcada mediante *window underflow*. Por eso NO hace
//!    falta almacenar a2..a15 en el TCB: viven en la pila de la tarea.
//!
//! 2. **Primer arranque** (`first_run == 1`): la tarea es nueva; `init_task_stack`
//!    dejó `entry`/`arg`/SP/PS preparados. En vez de `retw` (que exigiría fabricar
//!    a mano el marco de underflow — lo más frágil), hacemos una LLAMADA windowed
//!    normal `callx8` al trampolín, idéntica a la que emitiría el compilador. Así
//!    el handshake ventana/SP lo resuelve el propio `entry` del trampolín, que es
//!    justamente el contrato del ABI windowed.
//!
//! `switch_to`, al guardar la tarea saliente, pone su `first_run = 0`: tras el
//! primer arranque, toda reanudación posterior usa la ruta `retw`.
//!
//! ## AVISOS DE VALIDACIÓN EN HARDWARE (riesgo alto)
//!
//! Sin compilador ni placa a mano, estas partes se marcan como
//! `VALIDAR-HW` y deben comprobarse contra el ISA de Xtensa y contra
//! `esp-idf` (`components/xtensa/*.S`, macros `SPILL_ALL_WINDOWS`):
//!   * La secuencia `SPILL_ALL_WINDOWS` (rotación neta 0, requiere `PS.WOE=1`).
//!   * El uso de `callx8` para lanzar el trampolín (registro de argumento a10,
//!     continuidad de SP vía `entry`).
//!   * La ventana de una o dos instrucciones en que se rehabilitan interrupciones
//!     (`wsr.ps`) justo antes de saltar a la tarea.
//!   * Los nombres exactos de registros especiales (`ps`, `windowbase`,
//!     `windowstart`) tal y como los acepta el ensamblador del fork de Rust/Xtensa.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code, unused_imports)]

use core::arch::asm;
use core::mem::offset_of;

// Tipos compartidos del kernel. `context` no consume nada del prelude hoy, pero
// se importa por coherencia con el contrato (§3.2.1) y futuras necesidades.
use crate::prelude::*;

// ---------------------------------------------------------------------------
// Campos de bits de PS (Processor State). Fuente: Xtensa ISA / esp-hal.
// ---------------------------------------------------------------------------

/// `PS.INTLEVEL` ocupa los bits 3:0 (nivel de enmascarado de interrupciones).
const PS_INTLEVEL_MASK: u32 = 0x0000_000F;
/// `PS.EXCM` (bit 4): modo excepción. Debe ir a 0 en una tarea normal.
const PS_EXCM: u32 = 1 << 4;
/// `PS.UM` (bit 5): "User vector Mode". Operación normal del S3 = 1.
const PS_UM: u32 = 1 << 5;
/// `PS.WOE` (bit 18): "Window Overflow Enable". OBLIGATORIO=1 para código windowed
/// (y para que `SPILL_ALL_WINDOWS` funcione).
const PS_WOE: u32 = 1 << 18;

/// PS inicial de toda tarea nueva:
///   * `WOE=1`  -> ventanas de registros habilitadas (código windowed).
///   * `UM=1`   -> modo de operación normal.
///   * `EXCM=0` -> no estamos en una excepción.
///   * `INTLEVEL=0` -> interrupciones habilitadas: la tarea arranca con preempción
///     activa, tal y como asume `scheduler::task_trampoline`.
///
/// VALIDAR-HW: si el modelo de privilegios del kernel exigiera anillo distinto o
/// `UM=0`, ajustar aquí. Este es el único punto que fija el PS de arranque.
const INITIAL_PS: u32 = PS_WOE | PS_UM; // INTLEVEL=0, EXCM=0

/// Alineación de pila exigida por la ABI de Xtensa (16 bytes).
const STACK_ALIGN_MASK: usize = 0xF;

/// Reserva inicial bajo la cima de pila. Deja un "área base de guardado" de 16
/// bytes (para a0..a3 del primer marco si el trampolín se desborda) y mantiene la
/// alineación a 16.
const STACK_INITIAL_RESERVE: usize = 16;

// ---------------------------------------------------------------------------
// Contexto de CPU salvado por tarea.
// ---------------------------------------------------------------------------

/// Estado de CPU salvado de una tarea (Xtensa LX7 con ventanas).
///
/// `#[repr(C)]` es OBLIGATORIO: el `switch_to` en `asm!` accede a estos campos por
/// desplazamiento (calculado con `offset_of!`, nunca a mano), así que el orden y
/// el tamaño deben ser estables. Todos los campos son `u32` (Xtensa es de 32 bits)
/// y quedan naturalmente a offsets múltiplos de 4, como exigen `l32i`/`s32i`.
///
/// DECISIÓN DE DISEÑO: NO se almacenan `a2..a15`. En la ruta de reanudación esos
/// registros se reconstruyen por *window underflow* desde la pila de la tarea (que
/// se volcó al suspenderla). En la ruta de primer arranque se pasan por registro a
/// través de una llamada windowed normal. El TCB queda así mínimo y sin duplicar
/// estado que ya vive, canónicamente, en la pila.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    /// PC informativo/depuración de la tarea (no lo consume la ruta `retw`).
    pub pc: u32, // offset 0
    /// Processor State salvado (INTLEVEL, WOE, UM, ...).
    pub ps: u32, // offset 4
    /// `a1` = stack pointer en el punto de conmutación.
    pub sp: u32, // offset 8
    /// `a0` = dirección de retorno windowed hacia el planificador (ruta `retw`).
    pub a0: u32, // offset 12
    /// (Sólo primer arranque) puntero a la función de entrada (`fn(usize)`).
    pub entry: u32, // offset 16
    /// (Sólo primer arranque) argumento entregado a `entry`.
    pub arg: u32, // offset 20
    /// 1 = primer arranque (ruta `callx8`); 0 = reanudar (ruta `retw`).
    pub first_run: u32, // offset 24
}

// ---------------------------------------------------------------------------
// Preparación de la pila/contexto de una tarea nueva (LÓGICA PURA — real).
// ---------------------------------------------------------------------------

/// Prepara el `Context` inicial de una tarea nueva para que el primer `switch_to`
/// la lance en `entry(arg)`. [CANÓNICO]
///
/// * `stack_top` = dirección MÁS ALTA de la pila (la pila crece hacia abajo).
/// * `entry`     = función de la tarea; en la práctica el trampolín del scheduler.
/// * `arg`       = primer argumento de `entry` (el scheduler pasa el `tid`).
///
/// No ejecuta ensamblador: sólo compone los campos que consumirá la ruta de
/// "primer arranque" de `switch_to`. Es `#[inline(never)]` para que su marco no se
/// entremezcle con el del llamador (irrelevante para corrección, útil al depurar).
#[inline(never)]
pub fn init_task_stack(stack_top: *mut u8, entry: fn(usize), arg: usize) -> Context {
    // Alinear la cima a 16 bytes hacia ABAJO (nunca por encima de la región dada)
    // y reservar el área base de guardado. Se usa aritmética saturante para no
    // desbordar jamás en rutas del kernel.
    let top = (stack_top as usize) & !STACK_ALIGN_MASK;
    let sp = top.saturating_sub(STACK_INITIAL_RESERVE) & !STACK_ALIGN_MASK;

    // `fn(usize)` -> dirección. En Xtensa (32 bits) `usize` == `u32`; el `as u32`
    // no pierde información.
    let entry_addr = entry as usize as u32;

    Context {
        pc: entry_addr, // informativo: dónde empezará a ejecutar la tarea
        ps: INITIAL_PS, // interrupciones ON, ventanas ON
        sp: sp as u32,
        a0: 0, // sin retorno windowed válido aún (no se usa en primer arranque)
        entry: entry_addr,
        arg: arg as u32,
        first_run: 1, // marca: usar la ruta de lanzamiento (callx8)
    }
}

// ---------------------------------------------------------------------------
// Cambio de contexto (ENSAMBLADOR — mejor esfuerzo, VALIDAR-HW).
// ---------------------------------------------------------------------------

/// Salva el contexto actual en `*current` y restaura/lanza `*next`. [CANÓNICO]
///
/// # Safety
/// Manipula estado de CPU directamente. Sólo la invoca el planificador con las
/// interrupciones enmascaradas (o desde la ISR del tick). `current` y `next` deben
/// apuntar a `Context` válidos y ESTABLES durante toda la llamada (el scheduler los
/// mantiene en `Box`, por eso sus direcciones no cambian).
///
/// Comportamiento observable desde el llamador:
///   * Ruta de reanudación: `switch_to` "retorna" a este llamador MÁS TARDE, cuando
///     la tarea vuelva a planificarse (semántica que `scheduler::schedule` espera:
///     tras el retorno restaura las interrupciones).
///   * Ruta de primer arranque de `next`: `switch_to` NO retorna a este llamador;
///     transfiere el control al trampolín de la tarea nueva. El PS de la tarea pasa
///     a gobernar el nivel de interrupciones.
#[inline(never)]
pub unsafe fn switch_to(current: *mut Context, next: *const Context) {
    // Los argumentos se fijan en a2 (current) y a3 (next). Tras el `entry` que el
    // compilador emite para esta función, `a0` es la dirección de retorno windowed
    // hacia el planificador y `a1` el SP de este marco (ambos sobre la pila de la
    // tarea SALIENTE, que es donde corre `switch_to`).
    //
    // `options(noreturn)`: desde el punto de vista del compilador, el control no
    // cae tras el bloque `asm!` (sale por `retw`, por `callx8`+bucle, o se reanuda
    // dentro del planificador). Por eso NO se declaran operandos de salida ni
    // clobbers (incompatibles con `noreturn`): usamos registros fijos como scratch
    // (a4/a5) y no nos importa dejarlos sucios.
    asm!(
        // ================= GUARDAR TAREA SALIENTE (a2 = current) =================
        "rsr.ps  a4",                    // a4 <- PS actual
        "s32i    a4, a2, {O_PS}",        // current.ps  = PS
        "s32i    a0, a2, {O_A0}",        // current.a0  = retorno windowed al scheduler
        "s32i    a1, a2, {O_SP}",        // current.sp  = SP del marco de switch_to
        "movi    a4, 0",
        "s32i    a4, a2, {O_FIRST}",     // current.first_run = 0 (ya no es "nueva")

        // ================= VOLCAR TODAS LAS VENTANAS (spill) =====================
        // Secuencia canónica de esp-idf (`SPILL_ALL_WINDOWS`, XCHAL_NUM_AREGS=64).
        // Requiere PS.WOE=1. Rotación total 3+3+3+3+4 = 16 unidades = vuelta
        // completa: al terminar WINDOWBASE queda igual y a0..a15 conservan su valor
        // (el `and a12,a12,a12` es un no-op de valor que "toca" cada ventana para
        // forzar su volcado a la pila). Así a2 (current) y a3 (next) SOBREVIVEN.
        // VALIDAR-HW: mecanismo exacto de volcado y condición WOE.
        "and a12, a12, a12",
        "rotw 3",
        "and a12, a12, a12",
        "rotw 3",
        "and a12, a12, a12",
        "rotw 3",
        "and a12, a12, a12",
        "rotw 3",
        "and a12, a12, a12",
        "rotw 4",

        // ================= ¿PRIMER ARRANQUE DE next? (a3 = next) =================
        "l32i    a4, a3, {O_FIRST}",
        "beqz    a4, 2f",                // first_run == 0 -> reanudar (retw)

        // --------------- RUTA DE PRIMER ARRANQUE (callx8 al trampolín) ----------
        "l32i    a1, a3, {O_SP}",        // SP = pila nueva de la tarea
        "l32i    a4, a3, {O_PS}",
        "wsr.ps  a4",                    // PS = INITIAL_PS (rehabilita interrupciones)
        "rsync",
        // Dejar viva SÓLO la ventana actual: WINDOWSTART = 1 << WINDOWBASE. Descarta
        // los marcos del llamador (el planificador), a los que NO volveremos por
        // esta ruta, y evita desbordes espurios en el `callx8`.
        "movi    a4, 1",
        "rsr.windowbase a5",
        "ssl     a5",                    // SAR = 32 - windowbase
        "sll     a4, a4",                // a4 = 1 << windowbase
        "wsr.windowstart a4",
        "rsync",
        // Argumento en a10 (convención call8: a10 del llamador -> a2 del llamado).
        "l32i    a10, a3, {O_ARG}",
        // Dirección del trampolín en a8; leerla antes de la rotación del call.
        "l32i    a8,  a3, {O_ENTRY}",
        // Llamada windowed estándar: el `entry` del trampolín resuelve SP/ventana.
        // VALIDAR-HW: registro de destino de callx8 y continuidad de SP vía entry.
        "callx8  a8",
        // El trampolín no retorna (acaba en `scheduler::exit`). Por seguridad, si
        // regresara, quedamos en un bucle acotado en vez de caer en código ajeno.
        "1:",
        "j       1b",

        // --------------- RUTA DE REANUDACIÓN (retw) ----------------------------
        "2:",
        "l32i    a4, a3, {O_PS}",
        "wsr.ps  a4",                    // restaurar PS de la tarea entrante
        "rsync",
        "l32i    a0, a3, {O_A0}",        // dirección de retorno windowed guardada
        "l32i    a1, a3, {O_SP}",        // SP guardado
        // `retw` devuelve el control al llamador de switch_to de la tarea entrante,
        // reconstruyendo a0..a15 por window underflow desde SU pila volcada.
        // VALIDAR-HW: los 2 bits altos de a0 (incremento de ventana) deben ser los
        // que dejó el `call` original del planificador (los guardamos intactos).
        "retw",

        // ----------------------- Operandos / constantes ------------------------
        O_PS    = const offset_of!(Context, ps),
        O_SP    = const offset_of!(Context, sp),
        O_A0    = const offset_of!(Context, a0),
        O_FIRST = const offset_of!(Context, first_run),
        O_ARG   = const offset_of!(Context, arg),
        O_ENTRY = const offset_of!(Context, entry),
        in("a2") current,
        in("a3") next,
        options(noreturn),
    )
}
