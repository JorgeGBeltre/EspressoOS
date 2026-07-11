//! Sincronización entre núcleos / arranque de APP_CPU (ESQUELETO — Fase 9, SMP).
//!
//! El ESP32-S3 tiene dos núcleos Xtensa LX7: PRO_CPU (núcleo 0, el que arranca)
//! y APP_CPU (núcleo 1). En Fase 2 el kernel corre monoprocesador sobre PRO_CPU;
//! este módulo queda como gancho documentado para la evolución a SMP.
//!
//! Plan de Fase 9 (resumen de lo que implementará `start_secondary_core`):
//!   1. Reservar y preparar la pila del APP_CPU y su tabla de tareas por-core.
//!   2. Liberar el reset del segundo núcleo (registros del sistema:
//!      `SYSTEM_CORE_1_CONTROL_*` / equivalente en esp-hal) y fijar su punto de
//!      entrada (vector de arranque del APP_CPU).
//!   3. Sincronizar el arranque con una barrera (spinlock SMP-safe) hasta que el
//!      APP_CPU haya instalado su VECBASE y su temporizador local.
//!   4. Entregarle su propio bucle de planificación (colas por-core + balanceo /
//!      afinidad de tareas), reutilizando `scheduler::run` por núcleo.
//!
//! Requisitos que habilita esta fase en el resto del kernel:
//!   - Los locks del kernel deben ser SMP-safe de verdad (el `SpinLock` atómico
//!     de `arch::xtensa::sync` ya lo es; las secciones que hoy solo enmascaran
//!     interrupciones deberán además tomar el lock para excluir al otro núcleo).
//!   - El estado global del scheduler pasa de único a per-core, con migración de
//!     tareas coordinada.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

/// Arranca el segundo núcleo (APP_CPU). ESQUELETO (Fase 9). [CANÓNICO]
///
/// En Fase 2 es intencionadamente un no-op seguro: el kernel es monoprocesador y
/// no debe tocar el APP_CPU todavía. No panica ni bloquea; simplemente retorna
/// sin efecto para que el flujo de arranque monoprocesador sea correcto.
pub fn start_secondary_core() {
    // TODO(fase-9): liberar el reset del APP_CPU, fijar su entrada y darle su
    // propio bucle de scheduler (ver plan en la cabecera del módulo).
}
