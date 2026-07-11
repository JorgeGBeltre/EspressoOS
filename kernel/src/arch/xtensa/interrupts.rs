//! Manejo de interrupciones/excepciones del Xtensa LX7 (ESP32-S3) — Fase 3.
//!
//! IMPORTANTE — NO es RISC-V: aquí no existe `mtvec`. El Xtensa ubica sus
//! vectores con el registro especial `VECBASE` y una tabla de vectores POR
//! NIVEL de interrupción (window overflow/underflow, excepción de kernel y de
//! usuario, doble excepción, y un vector por cada nivel 1..N). El enmascarado
//! global se hace vía el registro `PS` (Processor State), campo `PS.INTLEVEL`
//! (4 bits): toda interrupción cuyo nivel sea <= INTLEVEL queda enmascarada.
//! El enmascarado por LÍNEA concreta se hace con el registro `INTENABLE`.
//!
//! Este módulo ofrece la capa CANÓNICA de secciones críticas del kernel
//! (`disable`/`restore`/`critical_section`). El resto del kernel (scheduler,
//! drivers, VFS...) enmascara SIEMPRE a través de aquí, nunca tocando `PS`/
//! `INTENABLE` a mano.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use core::arch::asm;

/// Nivel al que se sube `PS.INTLEVEL` en una sección crítica.
///
/// `15` es el máximo del campo (4 bits) y enmascara TODAS las interrupciones
/// enmascarables del LX7 (niveles 1..=7 en el S3). Es la misma elección que
/// hace `xtensa-lx::interrupt::disable`. Si en el futuro se quisieran mantener
/// vivas las interrupciones de alto nivel (p. ej. depuración/NMI de nivel 7),
/// se bajaría este valor a `XCHAL_EXCM_LEVEL` (== 3 en el S3); ojo: el operando
/// de `rsil` es INMEDIATO, así que además habría que cambiar el literal del
/// `asm!` de `disable`, no basta con esta constante.
const CRITICAL_INTLEVEL: u32 = 15;

/// Instala/prepara el manejo de interrupciones. Fase 3. [CANÓNICO]
///
/// DECISIÓN DE DISEÑO (riesgo alto si se ignora): la tabla de vectores y el
/// valor de `VECBASE` los fija el runtime `xtensa-lx-rt` (sobre el que se apoya
/// `esp-hal`) en el reset handler, y sus vectores viven ubicados por el linker
/// script. Esos vectores incluyen el manejo de OVERFLOW/UNDERFLOW de las
/// ventanas de registros: reapuntar `VECBASE` a una tabla propia INCOMPLETA
/// dejaría el spill/fill de ventanas sin manejar y colgaría el chip al primer
/// desbordamiento. Por eso aquí NO se sobrescribe `VECBASE`: se REUTILIZA la
/// tabla del runtime.
///
/// En consecuencia `init` es casi un no-op: no hace falta habilitar líneas aquí
/// porque cada driver (p. ej. `arch::xtensa::timer`) engancha su ISR y habilita
/// su propia línea (vía `INTENABLE`/matriz de interrupciones de `esp-hal`)
/// cuando se inicializa. Se mantiene la función como punto ÚNICO donde, si algún
/// día se instalara una tabla de vectores propia, se haría:
/// `wsr.vecbase <dir>; rsync`.
pub fn init() {
    // Punto de instalación de una tabla propia (deshabilitado a propósito).
    // Se deja el valor actual de VECBASE (el del runtime) intacto.
    let _vecbase_actual = read_vecbase();
    // Nada más que hacer en Fase 3: las líneas se habilitan por driver.
}

/// Enmascara TODAS las interrupciones enmascarables y devuelve el `PS` previo
/// (para reponerlo con `restore`). [CANÓNICO]
///
/// Emparejar SIEMPRE con `restore(estado)`. Es re-entrante/anidable: como el
/// token es el `PS` completo previo, anidar `disable`/`restore` restaura de
/// forma correcta el nivel exacto que hubiera en cada capa.
#[inline(always)]
pub fn disable() -> u32 {
    let ps_previo: u32;
    // `rsil at, level`  ("Read and Set Interrupt Level"): lee `PS` en `at` y fija
    // `PS.INTLEVEL = level` de forma ATÓMICA. Devolvemos el `PS` completo previo.
    //
    // Sin `options(nomem)` a propósito: el `asm!` actúa como barrera de memoria
    // para el compilador, impidiendo que reordene accesos del cuerpo de la
    // sección crítica por delante del enmascarado.
    unsafe {
        asm!("rsil {0}, 15", out(reg) ps_previo, options(nostack));
    }
    ps_previo
}

/// Repone el estado de interrupciones devuelto por `disable`. [CANÓNICO]
///
/// Escribe el `PS` completo guardado (que incluye el `INTLEVEL` previo) y
/// sincroniza con `rsync`, obligatorio tras escribir `PS` para que el nuevo
/// nivel surta efecto antes de la siguiente instrucción.
#[inline(always)]
pub fn restore(state: u32) {
    unsafe {
        asm!(
            "wsr.ps {0}",
            "rsync",
            in(reg) state,
            options(nostack),
        );
    }
}

/// Ejecuta `f` en sección crítica (interrupciones enmascaradas) y devuelve su
/// resultado. Patrón RAII manual sobre `disable`/`restore`. [CANÓNICO]
///
/// Nota: en este kernel `f` NO debe hacer panics (regla transversal §0.5); si
/// `f` hiciera un cambio de contexto (p. ej. el scheduler), es responsabilidad
/// del llamante gestionar el estado de interrupciones — por eso el scheduler
/// llama a `disable`/`restore` a mano en sus rutas de `switch_to` en lugar de
/// usar este envoltorio.
#[inline]
pub fn critical_section<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let estado = disable();
    let resultado = f();
    restore(estado);
    resultado
}

/// Lee el registro especial `VECBASE` (diagnóstico). Útil para verificar que la
/// tabla de vectores del runtime está donde se espera.
#[inline(always)]
fn read_vecbase() -> u32 {
    let vecbase: u32;
    // `rsr.vecbase at`: Read Special Register VECBASE.
    unsafe {
        asm!("rsr.vecbase {0}", out(reg) vecbase, options(nostack, nomem, preserves_flags));
    }
    vecbase
}
