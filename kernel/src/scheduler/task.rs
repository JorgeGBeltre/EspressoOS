//! Estructura de tarea (TCB) y su ciclo de vida (Fase 2).
//!
//! Cada tarea posee su propia pila asignada en el heap y un `Context` que
//! `arch::xtensa::context::switch_to` guarda/restaura. La pila se libera de
//! forma determinista en el `Drop` del `Task` (al reaparear una tarea zombie).
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use core::alloc::Layout;

use crate::arch::xtensa::context::{self, Context};
use crate::prelude::*;

/// Identificador de tarea. [CANÓNICO]
pub type Tid = u32;

/// Estado de una tarea en el planificador. [CANÓNICO]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Zombie,
}

/// Alineación de pila exigida por la ABI de Xtensa (16 bytes).
const STACK_ALIGN: usize = 16;

/// Redondea `n` hacia arriba al múltiplo de `align` (potencia de 2).
/// Usa `saturating_add` para no desbordar nunca en rutas del kernel.
const fn align_up(n: usize, align: usize) -> usize {
    n.saturating_add(align - 1) & !(align - 1)
}

/// Bloque de control de tarea (TCB). [CANÓNICO]
///
/// Los campos públicos son los del contrato. `start_entry`/`start_arg` son
/// bookkeeping interno del scheduler (los usa el trampolín de arranque) y por
/// eso quedan restringidos con `pub(super)`.
pub struct Task {
    pub tid: Tid,
    pub name: String, // nombre corto (p. ej. "shell")
    pub state: TaskState,
    pub priority: u8, // 0 = más baja
    pub context: Context,
    pub stack_base: *mut u8, // base de la pila (dirección más baja)
    pub stack_size: usize,   // tamaño real asignado (alineado)
    pub exit_code: i32,      // válido cuando state == Zombie

    /// Función de entrada real de la tarea (la ejecuta el trampolín).
    pub(super) start_entry: fn(usize),
    /// Argumento que recibe `start_entry`.
    pub(super) start_arg: usize,
}

// SAFETY: el único campo no-`Send` es `stack_base` (*mut u8). Cada `Task` es
// dueño EXCLUSIVO de su pila; el puntero no se aliasa ni se comparte fuera de la
// tarea, y todo acceso a la tabla de tareas está serializado por el lock del
// scheduler (con interrupciones enmascaradas). Trasladar un `Task` entre
// contextos es, por tanto, seguro.
unsafe impl Send for Task {}

impl Task {
    /// Crea una tarea nueva: asigna su pila en el heap y prepara el `Context`
    /// inicial para que el primer `switch_to` arranque en el trampolín del
    /// scheduler. Devuelve un `Box<Task>` para que la dirección del `Context`
    /// sea estable (los punteros crudos que consume `switch_to` deben seguir
    /// siendo válidos aunque la tabla de tareas mute).
    pub(super) fn new(
        tid: Tid,
        name: &str,
        entry: fn(usize),
        arg: usize,
        stack_size: usize,
        priority: u8,
    ) -> KResult<Box<Task>> {
        // Tamaño efectivo: si se pide 0, usar el tamaño por defecto del layout.
        let requested = if stack_size == 0 {
            layout::DEFAULT_STACK_SIZE
        } else {
            stack_size
        };
        let size = align_up(requested, STACK_ALIGN);
        let alloc_layout =
            Layout::from_size_align(size, STACK_ALIGN).map_err(|_| KError::InvalidArgument)?;

        // SAFETY: `alloc_layout` tiene tamaño > 0 y alineación válida.
        let base = unsafe { alloc::alloc::alloc(alloc_layout) };
        if base.is_null() {
            return Err(KError::NoMem);
        }

        // Cima de la pila (dirección MÁS ALTA); la pila crece hacia abajo.
        // SAFETY: `base .. base+size` es la región recién asignada.
        let stack_top = unsafe { base.add(size) };

        // El trampolín (`super::task_trampoline`) recibe el propio tid como
        // argumento; desde él recupera `start_entry`/`start_arg` y los ejecuta.
        let context = context::init_task_stack(stack_top, super::task_trampoline, tid as usize);

        Ok(Box::new(Task {
            tid,
            name: String::from(name),
            state: TaskState::Ready,
            priority,
            context,
            stack_base: base,
            stack_size: size,
            exit_code: 0,
            start_entry: entry,
            start_arg: arg,
        }))
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if !self.stack_base.is_null() && self.stack_size > 0 {
            // Reconstruir EXACTAMENTE el layout usado en `new`.
            if let Ok(dealloc_layout) = Layout::from_size_align(self.stack_size, STACK_ALIGN) {
                // SAFETY: liberamos la misma región asignada en `new` con idéntico
                // layout. En este punto la tarea es zombie y ya no corre sobre su
                // pila (el scheduler cambió a otra antes de reapearla).
                unsafe { alloc::alloc::dealloc(self.stack_base, dealloc_layout) };
            }
        }
    }
}
