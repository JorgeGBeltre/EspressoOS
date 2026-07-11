//! Política de planificación (Fase 2).
//!
//! Implementa round-robin real sobre la cola de listas (`Scheduler::ready`),
//! que es una FIFO de tids: se elige el frente y las tareas reencoladas por el
//! planificador van al fondo. Más adelante se evolucionará a prioridades fijas
//! o colas multinivel.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use super::task::Tid;
use super::Scheduler;

/// Extrae la siguiente tarea lista de la cola (round-robin). Interno: opera
/// sobre un `&mut Scheduler` ya bloqueado por el planificador, evitando así
/// re-tomar el lock (que no es reentrante).
///
/// `Some(tid)` = próxima a ejecutar; `None` = no hay tareas listas (el
/// planificador recurrirá a la tarea idle).
pub(super) fn next_ready(sched: &mut Scheduler) -> Option<Tid> {
    if sched.ready.is_empty() {
        None
    } else {
        // `remove(0)` es seguro: la cola no está vacía. O(n) es aceptable para el
        // número reducido de tareas de este kernel.
        Some(sched.ready.remove(0))
    }
}

/// Elige la siguiente tarea Ready. Round-robin en Fase 2. [CANÓNICO]
///
/// Variante pública (sin argumentos) del contrato: bloquea el estado global y
/// delega en `next_ready`. La lógica interna del planificador usa `next_ready`
/// directamente para no anidar tomas de lock.
pub fn pick_next() -> Option<Tid> {
    super::with_sched(next_ready).flatten()
}
