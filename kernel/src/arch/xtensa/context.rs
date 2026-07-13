//! Cambio de contexto en Xtensa LX7 (ESP32-S3, windowed ABI) — la pieza MÁS
//! delicada del kernel. Reimplementación CANÓNICA estilo FreeRTOS-Xtensa/esp-idf.
//!
//! El Xtensa LX7 usa *registros con ventana*: 64 registros físicos (AR0..AR63)
//! de los que en cada instante se ven 16 (`a0..a15`), situados por `WINDOWBASE`.
//! `WINDOWSTART` es un mapa de bits que marca qué grupos de 4 registros son el
//! inicio de un marco de llamada vivo. Un cambio de contexto correcto DEBE
//! volcar (spill) todas las ventanas vivas de la tarea saliente a SU pila, para
//! poder reconstruirlas por *window underflow* al reanudarla.
//!
//! ## Modelo (dos rutas, discriminadas por `first_run` de la tarea entrante)
//!
//! 1. **Reanudación** (`first_run == 0`): la tarea ya corrió y se suspendió
//!    DENTRO de su propio `switch_to`. Guardamos lo mínimo (`a0` = retorno
//!    windowed al planificador, `a1` = SP, `ps`), volcamos todas las ventanas y
//!    salimos con `retw`. El hardware reconstruye `a0..a15` del marco del
//!    planificador entrante por *window underflow* desde SU pila (que se volcó al
//!    suspenderla). Por eso el `Context` NO guarda `a2..a15`.
//!
//! 2. **Primer arranque** (`first_run == 1`): la tarea es nueva. `init_task_stack`
//!    fabricó en la cima de su pila un MARCO DE EXCEPCIÓN `XT_STK` (PC = trampolín,
//!    arg en `a6`, `a1` = SP físico, `PS = UM|EXCM|WOE|CALLINC(1)`). `switch_to`
//!    restaura ese marco y salta con `rfe` — la MISMA ruta canónica por la que el
//!    hardware reanuda una tarea preemptada. `rfe` limpia `EXCM` y salta a `EPC1`
//!    de forma ATÓMICA (las interrupciones quedan enmascaradas hasta el trampolín).
//!    **No se usa `callx8`/`jx`**: el `entry` del propio trampolín resuelve SP y
//!    ventana, y —por `CALLINC(1)`— mapea `a6` (pre-entry) a `a2` (post-entry) =
//!    primer argumento de `fn(usize)`.
//!
//! `switch_to`, al guardar la tarea saliente, pone su `first_run = 0`: tras el
//! primer arranque, toda reanudación posterior usa la ruta `retw`.
//!
//! ## Fallo raíz corregido respecto a la versión previa
//! El spill (`SPILL_ALL_WINDOWS`) NO es aritmética: PROVOCA excepciones de
//! window-overflow cuyos *handlers* vuelcan las ventanas a la pila y limpian
//! `WINDOWSTART`. Para que esas excepciones se tomen hacen falta PRECONDICIONES
//! que la versión previa nunca fijaba: `PS.EXCM=0`, `PS.WOE=1` e
//! `INTLEVEL >= XCHAL_EXCM_LEVEL(3)`, más salvar/restaurar `EPC1` (las overflow lo
//! pisan). Sin ellas el spill era un *silent no-op*, `WINDOWSTART` no se reseteaba
//! y el `retw`/`callx8` posterior operaba sobre ventanas con basura -> CUELGUE.
//! Aquí se establecen esas precondiciones y, además, se fuerza
//! `WINDOWSTART = 1<<WINDOWBASE` de forma DEFENSIVA en ambas rutas.
//!
//! ## Referencia del mecanismo
//!   * `SPILL_ALL_WINDOWS` (esp-idf `xt_asm_utils.h`, variante 64 AREGS) y sus
//!     precondiciones (`components/xtensa/xtensa_context.S`): PS con `EXCM=0`,
//!     `WOE=1`, `INTLEVEL>=3`; `EPC1` salvado en `a0` (registro cuya supervivencia
//!     al spill usa esp-idf) y restaurado.
//!   * `pxPortInitialiseStack` windowed (arg en `a6`, `PS_CALLINC(1)`, marco
//!     zero-inicializado) — port clásico Cadence/FreeRTOS-Xtensa.
//!   * Semántica ISA de `entry`/`retw`/`rfe`, `WINDOWBASE`/`WINDOWSTART` — Xtensa
//!     ISA Reference (WindowedOption).
//!
//! ## AVISOS DE VALIDACIÓN EN HARDWARE (VALIDAR-HW)
//!   * Requiere los vectores de window over/underflow instalados (xtensa-lx-rt /
//!     esp-hal los instala en boot). Sin ellos el spill es un no-op y el
//!     `retw`/`entry` fallan.
//!   * Offsets `XT_STK_*` y `XT_STK_FRMSZ` dependen de la config del core; aquí
//!     sólo se fabrican/leen PC/PS/A0/A1/A6, cuyos offsets son estables, pero
//!     confirmar `FRMSZ`/alineación contra el build.
//!   * Supervivencia de `a2`/`a3`/`a0` al `SPILL_ALL_WINDOWS` (rotación neta 0 en
//!     64 AREGS); es lo que esp-idf da por bueno (usa `a0` para `EPC1`, `a2` para
//!     `PS`).
//!   * Ventana entre `wsr.ps` (EXCM=1) y `rfe`/`retw`: interrupciones de nivel >
//!     `XCHAL_EXCM_LEVEL` (4/5 en S3) podrían entrar; riesgo teórico común a todo
//!     conmutador windowed.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code, unused_imports)]

use core::arch::asm;
use core::mem::offset_of;

// Tipos compartidos del kernel. `context` no consume nada del prelude hoy, pero
// se importa por coherencia con el contrato y futuras necesidades.
use crate::prelude::*;

// ---------------------------------------------------------------------------
// Campos de bits de PS (Processor State). Fuente: Xtensa ISA / esp-idf.
// ---------------------------------------------------------------------------

/// `PS.INTLEVEL` ocupa los bits 3:0 (nivel de enmascarado de interrupciones).
const PS_INTLEVEL_MASK: u32 = 0x0000_000F;
/// `PS.EXCM` (bit 4): modo excepción. En una tarea EN EJECUCIÓN debe ser 0; en el
/// marco fabricado va a 1 porque `rfe` lo limpiará al lanzar la tarea.
const PS_EXCM: u32 = 1 << 4;
/// `PS.UM` (bit 5): "User vector Mode". Operación normal del S3 = 1.
const PS_UM: u32 = 1 << 5;
/// `PS.CALLINC` (bits 17:16): incremento de ventana del próximo `entry`. Con
/// `CALLINC(1)` el `entry` del trampolín rota +4 registros, de modo que el `a6`
/// pre-entry se convierte en el `a2` (primer argumento) post-entry.
const PS_CALLINC1: u32 = 1 << 16;
/// `PS.WOE` (bit 18): "Window Overflow Enable". OBLIGATORIO=1 para código windowed
/// y para que `SPILL_ALL_WINDOWS` provoque las excepciones de overflow.
const PS_WOE: u32 = 1 << 18;

/// Nivel de interrupción de excepción del LX7 (ESP32-S3). El spill exige subir
/// `INTLEVEL` al menos a este valor mientras `EXCM=0`.
const XCHAL_EXCM_LEVEL: u32 = 3;

/// PS del MARCO fabricado de una tarea nueva:
///   `UM | EXCM | WOE | CALLINC(1)`  ( = 0x5_0030 ).
/// El `EXCM=1` es intencional: durante la restauración corremos "en modo
/// excepción" (enmascarado) y **`rfe` limpia `EXCM`** al saltar. La tarea acaba
/// arrancando con `EXCM=0, INTLEVEL=0, UM=1, WOE=1, CALLINC=1`.
const FRAME_INITIAL_PS: u32 = PS_UM | PS_EXCM | PS_WOE | PS_CALLINC1;

/// PS "en ejecución" de una tarea (informativo; el que gobierna tras `rfe`):
/// `UM | WOE`, `EXCM=0`, `INTLEVEL=0` (preempción activa).
const INITIAL_PS: u32 = PS_UM | PS_WOE;

/// Alineación de pila exigida por la ABI de Xtensa (16 bytes).
const STACK_ALIGN_MASK: usize = 0xF;

// ---------------------------------------------------------------------------
// Offsets del marco de excepción `XT_STK` (esp-idf `xtensa_context.h`).
// Todos los campos son `long` (4 B). Sólo fabricamos/leemos el subconjunto que
// consume la ruta de primer arranque; el resto del marco queda a cero (lo
// zero-inicializa `init_task_stack`, porque el asignador de pila NO lo hace).
// ---------------------------------------------------------------------------

/// Despachador de salida (no usado por nuestro trampolín, que no retorna).
const XT_STK_EXIT: usize = 0x00;
/// PC de arranque de la tarea (entrada del trampolín). Va a `EPC1` y `rfe` salta.
const XT_STK_PC: usize = 0x04;
/// PS del marco (`FRAME_INITIAL_PS`).
const XT_STK_PS: usize = 0x08;
/// `a0` de la tarea (=0: corta el backtrace de GDB / retorno "a la nada").
const XT_STK_A0: usize = 0x0C;
/// `a1` = SP FÍSICO (tope del marco); el `entry` del trampolín deriva su SP de él.
const XT_STK_A1: usize = 0x10;
/// `a6` = argumento; tras el `entry` (CALLINC=1) pasa a ser `a2` = primer arg.
const XT_STK_A6: usize = 0x24;
/// `SAR` (shift amount). Irrelevante al arrancar; se deja a 0.
const XT_STK_SAR: usize = 0x4C;

/// Tamaño del marco `XT_STK` que reservamos (múltiplo de 16). Cubre hasta `SAR`
/// (0x4C) redondeado hacia arriba a 16 B. La tarea arrancará con su SP en el tope
/// de este marco; como el marco ya se consumió (sus datos están en registros tras
/// el `rfe`), la tarea puede reutilizar esa memoria al crecer su pila.
const XT_STK_FRMSZ: usize = 0x50;

// ---------------------------------------------------------------------------
// Contexto de CPU salvado por tarea.
// ---------------------------------------------------------------------------

/// Estado de CPU salvado de una tarea (Xtensa LX7 con ventanas).
///
/// `#[repr(C)]` es OBLIGATORIO: `switch_to` accede a estos campos por
/// desplazamiento (`offset_of!`), así que el orden y el tamaño deben ser
/// estables. Todos los campos son `u32` y quedan a offsets múltiplos de 4, como
/// exigen `l32i`/`s32i`.
///
/// DEBE seguir siendo `Copy + Default + repr(C)`: el planificador hace
/// `Context::default()` para el bootstrap desechable y lo guarda por valor dentro
/// de un `Box<Task>` (dirección estable). NUNCA accede a los campos por nombre, así
/// que su layout interno es libre mientras se respete ese contrato.
///
/// DECISIÓN DE DISEÑO: NO se almacenan `a2..a15`. En la reanudación se
/// reconstruyen por *window underflow* desde la pila de la tarea; en el primer
/// arranque viven en el marco `XT_STK` fabricado en la pila. `sp` tiene doble uso:
/// en reanudación es el SP guardado; en primer arranque es la dirección del marco
/// `XT_STK` fabricado.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    /// Processor State salvado (sólo ruta de reanudación).
    pub ps: u32, // offset 0
    /// `a1` = SP salvado (reanudación) / dirección del marco `XT_STK` (arranque).
    pub sp: u32, // offset 4
    /// `a0` = dirección de retorno windowed hacia el planificador (reanudación).
    pub a0: u32, // offset 8
    /// 1 = primer arranque (ruta `rfe`); 0 = reanudar (ruta `retw`).
    pub first_run: u32, // offset 12
}

// ---------------------------------------------------------------------------
// Preparación de la pila/contexto de una tarea nueva (LÓGICA PURA — real).
// ---------------------------------------------------------------------------

/// Escribe `val` en `base + off` de forma volátil (memoria compartida con el
/// ensamblador de `switch_to`, invisible para el optimizador).
#[inline(always)]
unsafe fn frame_put(base: usize, off: usize, val: u32) {
    // SAFETY: `base` es 16-alineado y `off` múltiplo de 4 -> dirección 4-alineada
    // dentro del buffer de pila recién asignado por el llamador.
    unsafe { ((base + off) as *mut u32).write_volatile(val) };
}

/// Prepara el `Context` inicial de una tarea nueva fabricando un MARCO DE
/// EXCEPCIÓN `XT_STK` en la cima de su pila, para que el primer `switch_to` la
/// lance por `rfe` ejecutando `entry(arg)`. [CANÓNICO]
///
/// * `stack_top` = dirección MÁS ALTA de la pila (una-más-allá del buffer; la
///   pila crece hacia abajo).
/// * `entry`     = función de la tarea; en la práctica `scheduler::task_trampoline`.
/// * `arg`       = primer argumento de `entry` (el scheduler pasa el `tid`).
///
/// Es `#[inline(never)]` para que su marco no se entremezcle con el del llamador
/// (irrelevante para corrección, útil al depurar).
#[inline(never)]
pub fn init_task_stack(stack_top: *mut u8, entry: fn(usize), arg: usize) -> Context {
    // Cima alineada a 16 hacia ABAJO (nunca por encima de la región dada).
    let top = (stack_top as usize) & !STACK_ALIGN_MASK;
    // Base del marco `XT_STK`: `top - FRMSZ`, 16-alineado. Como `top` es
    // 16-alineado y `FRMSZ` es múltiplo de 16, `frame_base` queda 16-alineado.
    let frame_base = top.saturating_sub(XT_STK_FRMSZ) & !STACK_ALIGN_MASK;
    // SP FÍSICO de la tarea = tope del marco (= `top`). El `entry` del trampolín
    // derivará su propio SP de aquí (a1 - framesize).
    let a1_physical = frame_base + XT_STK_FRMSZ;

    // `fn(usize)` -> dirección. En Xtensa (32 bits) `usize == u32`; sin pérdida.
    let entry_addr = entry as usize as u32;

    // Fabricar el marco `XT_STK` en la pila de la tarea. El asignador de pila NO
    // devuelve memoria a cero, así que ZERO-INICIALIZAMOS todo el marco (como
    // `pxPortInitialiseStack`) y luego escribimos los campos vivos. Zerar el marco
    // garantiza estado determinista para los slots `a*` que no fijamos.
    // SAFETY: `frame_base .. frame_base + FRMSZ` cae dentro del buffer de pila
    // (por debajo de `stack_top`) que el llamador acaba de asignar y posee.
    unsafe {
        let words = XT_STK_FRMSZ / 4;
        let mut i = 0;
        while i < words {
            frame_put(frame_base, i * 4, 0);
            i += 1;
        }
        frame_put(frame_base, XT_STK_EXIT, 0); // sin despachador de salida
        frame_put(frame_base, XT_STK_PC, entry_addr); // PC -> EPC1 (rfe)
        frame_put(frame_base, XT_STK_PS, FRAME_INITIAL_PS); // UM|EXCM|WOE|CALLINC(1)
        frame_put(frame_base, XT_STK_A0, 0); // a0 = 0 (corta backtrace)
        frame_put(frame_base, XT_STK_A1, a1_physical as u32); // SP físico
        frame_put(frame_base, XT_STK_A6, arg as u32); // arg -> a2 tras el entry
        frame_put(frame_base, XT_STK_SAR, 0); // SAR limpio
    }

    Context {
        ps: FRAME_INITIAL_PS, // informativo (la ruta de arranque lo lee del marco)
        sp: frame_base as u32, // dirección del marco `XT_STK` a restaurar
        a0: 0,                 // sin retorno windowed aún (no se usa en arranque)
        first_run: 1,          // marca: usar la ruta `rfe`
    }
}

// ---------------------------------------------------------------------------
// Cambio de contexto (ENSAMBLADOR — VALIDAR-HW).
// ---------------------------------------------------------------------------

/// Salva el contexto actual en `*current` y restaura/lanza `*next`. [CANÓNICO]
///
/// # Safety
/// Manipula estado de CPU directamente. Sólo la invoca el planificador con las
/// interrupciones enmascaradas (o desde la ISR del tick). `current` y `next`
/// deben apuntar a `Context` válidos y ESTABLES durante toda la llamada (el
/// scheduler los mantiene en `Box`).
///
/// Comportamiento observable:
///   * Reanudación de `next`: `switch_to` "retorna" a ESTE llamador MÁS TARDE,
///     cuando la tarea saliente vuelva a planificarse (semántica que
///     `scheduler::schedule` espera: tras el retorno restaura interrupciones).
///   * Primer arranque de `next`: `switch_to` NO retorna aquí; transfiere el
///     control al trampolín de la tarea nueva vía `rfe`.
///
/// En el PRIMER switch, `current` es un `Context::default()` desechable
/// (bootstrap) que jamás se reanuda: escribir en él es inocuo.
#[inline(never)]
pub unsafe fn switch_to(current: *mut Context, next: *const Context) {
    // Al ser `#[inline(never)]` (no `naked`), el compilador ya emitió el `entry`
    // de esta función: por eso al empezar el `asm!`, `a0` = retorno windowed al
    // scheduler (con sus 2 bits altos = CALLINC del `call`) y `a1` = SP de este
    // marco. Entradas fijas: `a2` = current, `a3` = next.
    //
    // `options(noreturn)`: el control NO cae tras el bloque (sale por `retw` o
    // `rfe`). No se declaran outputs ni clobbers (incompatibles con noreturn):
    // usamos registros fijos como scratch (a4..a8) y los dejamos sucios. `a2`/`a3`
    // y `a0` SOBREVIVEN al spill (rotación neta 0; esp-idf usa a0 para EPC1).
    unsafe {
        asm!(
            // ============ 1) GUARDAR TAREA SALIENTE (a2 = current) ============
            "rsr.ps  a4",                 // a4 <- PS actual
            "s32i    a4, a2, {O_PS}",     // current.ps = PS
            "s32i    a0, a2, {O_A0}",     // current.a0 = retorno windowed al scheduler
            "s32i    a1, a2, {O_SP}",     // current.sp = SP del marco de switch_to
            "movi    a4, 0",
            "s32i    a4, a2, {O_FIRST}",  // current.first_run = 0 (ya no es "nueva")

            // ==== 2) PRECONDICIONES DEL SPILL (EXCM=0, WOE=1, INTLEVEL>=3) ====
            // Sin esto, `SPILL_ALL_WINDOWS` es un no-op silencioso: NO vuelca las
            // ventanas ni resetea WINDOWSTART -> el `retw`/`rfe` posterior opera
            // sobre basura y CUELGA. (Este era el fallo raíz del código anterior.)
            "rsr.ps  a8",                 // PS de entrada (sólo para leer INTLEVEL)
            "extui   a5, a8, 0, 4",       // a5 = INTLEVEL actual (bits 3:0)
            "bgeui   a5, 3, 1f",          // max(INTLEVEL, XCHAL_EXCM_LEVEL=3)
            "movi    a5, 3",
            "1:",
            // Construir UM|WOE sin `movi` grande (evita literal pool en asm).
            "movi    a4, 1",
            "slli    a6, a4, 18",         // a6 = PS_WOE  (1<<18)
            "slli    a4, a4, 5",          // a4 = PS_UM   (1<<5)
            "or      a4, a4, a6",         // a4 = UM | WOE
            "or      a5, a5, a4",         // a5 = INTLEVEL | UM | WOE  (EXCM=0)
            "wsr.ps  a5",
            "rsync",
            // EPC1 lo pisan las excepciones de overflow del spill: salvarlo. Se usa
            // `a0` (su valor original ya está en current.a0), registro cuya
            // supervivencia al spill usa el propio esp-idf.
            "rsr.epc1 a0",                // salvar EPC1 en a0

            // ==== 3) SPILL_ALL_WINDOWS (64 AREGS) — rotación neta 3+3+3+3+4=16 ==
            // Vuelta completa: WINDOWBASE vuelve al origen y a2/a3/a0 sobreviven.
            // Cada `and a12,a12,a12` "toca" un registro que aún pertenece a un
            // marco padre -> dispara window-overflow -> su handler vuelca ese
            // marco a la pila y limpia su bit en WINDOWSTART. Al terminar,
            // WINDOWSTART == 1<<WINDOWBASE (sólo la ventana actual viva).
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
            "wsr.epc1 a0",                // restaurar EPC1
            "rsync",

            // ============ 4) ¿next es nuevo? (a3 = next) ============
            "l32i    a4, a3, {O_FIRST}",
            "bnez    a4, 2f",             // first_run != 0 -> arranque nuevo (rfe)

            // ---------------- 5) REANUDACIÓN (retw) ----------------
            "l32i    a4, a3, {O_PS}",     // PS guardado de la tarea entrante
            "l32i    a0, a3, {O_A0}",     // retorno windowed (bits CALLINC intactos)
            "l32i    a1, a3, {O_SP}",     // SP guardado de la tarea entrante
            "wsr.ps  a4",
            "rsync",
            // Defensivo: forzar WINDOWSTART = 1<<WINDOWBASE para GARANTIZAR el
            // underflow del retw aunque el spill fallara parcialmente. Usa a4/a5
            // como scratch (a0/a1 ya cargados y NO se tocan aquí).
            "movi    a4, 1",
            "rsr.windowbase a5",
            "ssl     a5",                 // SAR = 32 - windowbase
            "sll     a4, a4",             // a4 = 1 << windowbase
            "wsr.windowstart a4",
            "rsync",
            // `retw`: toma los 2 bits altos de a0 (CALLINC del `call` original),
            // hace WINDOWBASE -= n; como el bit destino de WINDOWSTART está a 0,
            // se dispara window-underflow -> recarga a0..a15 del marco del
            // scheduler entrante desde SU pila (base save area en [a1-16]). Vuelve
            // al llamador de switch_to de la tarea entrante.
            "retw",

            // ---------------- 6) PRIMER ARRANQUE (rfe) ----------------
            "2:",
            "l32i    a1, a3, {O_SP}",     // a1 = base del marco XT_STK fabricado
            "l32i    a5, a1, {XT_PS}",
            "wsr.ps  a5",                 // PS con EXCM=1 (enmascarado durante la carga)
            "l32i    a5, a1, {XT_PC}",
            "wsr.epc1 a5",                // EPC1 = entrada de la tarea; rfe saltará aquí
            "rsync",
            // Defensivo: WINDOWSTART = 1<<WINDOWBASE, para que el `entry` del
            // trampolín no dispare un overflow espurio contra ventanas obsoletas.
            "movi    a4, 1",
            "rsr.windowbase a5",
            "ssl     a5",
            "sll     a4, a4",
            "wsr.windowstart a4",
            "rsync",
            // Cargar la ventana pre-entry: a6=arg (-> a2 tras el entry), a0=0 (del
            // marco), y a4=0 para que el a0 POST-entry (= a4 pre-entry, por la
            // rotación +4 de CALLINC=1) también sea 0 y corte el backtrace.
            "movi    a4, 0",
            "l32i    a6, a1, {XT_A6}",    // arg -> a6
            "l32i    a0, a1, {XT_A0}",    // a0 = 0
            "l32i    a1, a1, {XT_A1}",    // a1 = SP físico (LEER a1 EL ÚLTIMO)
            "rfe",                        // limpia EXCM, PC<-EPC1: entra en el trampolín

            // ----------------------- Operandos / constantes ------------------
            O_PS    = const offset_of!(Context, ps),
            O_SP    = const offset_of!(Context, sp),
            O_A0    = const offset_of!(Context, a0),
            O_FIRST = const offset_of!(Context, first_run),
            XT_PS   = const XT_STK_PS,
            XT_PC   = const XT_STK_PC,
            XT_A0   = const XT_STK_A0,
            XT_A1   = const XT_STK_A1,
            XT_A6   = const XT_STK_A6,
            in("a2") current,
            in("a3") next,
            options(noreturn),
        )
    }
}
