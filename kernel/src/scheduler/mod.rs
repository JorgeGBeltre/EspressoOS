//! Planificador de tareas del kernel (Fase 2).
//!
//! Ofrece multitarea cooperativa (`yield_now`) y preemptiva (`tick`) sobre una
//! política round-robin (`policy`). Mantiene la tabla de tareas y la cola de
//! listas, crea la tarea `idle` y arranca el bucle con `run`. El cambio de
//! contexto REAL se delega íntegramente en `arch::xtensa::context::switch_to`
//! (aquí solo se invoca, nunca se implementa).
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use alloc::collections::BTreeMap;

use crate::arch::xtensa::context::{self, Context};
use crate::arch::xtensa::interrupts;
use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

pub mod core_sync;
pub mod policy;
pub mod task;

use task::{Task, TaskState, Tid};

/// Ticks de reloj que dura el quantum de una tarea antes de la preempción.
/// A `arch::xtensa::timer::TICK_HZ = 100`, 5 ticks ≈ 50 ms.
const QUANTUM_TICKS: u32 = 5;

/// Tid reservado para la tarea idle (siempre presente tras `init`).
const IDLE_TID: Tid = 0;

/// Estado global del planificador. Vive tras un `Mutex` y SIEMPRE se accede con
/// las interrupciones enmascaradas (ver `with_sched` y `schedule`), para que la
/// ISR del tick no pueda re-entrar mientras se manipula.
struct Scheduler {
    /// Todas las tareas por tid. Se guardan en `Box` para que la dirección de su
    /// `Context` sea ESTABLE: los punteros crudos que recibe `switch_to` deben
    /// seguir siendo válidos aun después de soltar el lock del scheduler.
    tasks: BTreeMap<Tid, Box<Task>>,
    /// Cola FIFO de tareas listas (round-robin). No incluye ni a la tarea actual
    /// ni a la tarea idle.
    ready: Vec<Tid>,
    /// Tarea en ejecución.
    current: Tid,
    /// Tarea idle (fallback cuando no hay nada listo).
    idle: Tid,
    /// Próximo tid a asignar.
    next_tid: Tid,
    /// Ticks restantes del quantum de la tarea actual.
    slice_remaining: u32,
}

impl Scheduler {
    /// Puntero (const) al `Context` de una tarea, estable por vivir en `Box`.
    fn ctx_ptr(&self, tid: Tid) -> Option<*const Context> {
        self.tasks.get(&tid).map(|t| &t.context as *const Context)
    }

    /// Puntero mutable al `Context` de una tarea.
    fn ctx_ptr_mut(&mut self, tid: Tid) -> Option<*mut Context> {
        self.tasks
            .get_mut(&tid)
            .map(|t| &mut t.context as *mut Context)
    }

    /// Libera las tareas Zombie (excepto `keep`), devolviendo su pila al heap.
    /// `keep` es la tarea actual: aunque sea zombie, todavía corre sobre su pila,
    /// así que no debe liberarse hasta que el scheduler cambie a otra.
    fn reap_zombies_except(&mut self, keep: Tid) {
        let mut dead: Vec<Tid> = Vec::new();
        for (tid, t) in self.tasks.iter() {
            if t.state == TaskState::Zombie && *tid != keep {
                dead.push(*tid);
            }
        }
        for tid in dead {
            self.ready.retain(|x| *x != tid);
            // El `Drop` de `Task` libera la pila asociada.
            self.tasks.remove(&tid);
        }
    }
}

/// Instancia global del planificador (`None` hasta `init`).
static SCHED: Mutex<Option<Scheduler>> = Mutex::new(None);

/// Ejecuta `f` con acceso mutable al planificador y las interrupciones
/// enmascaradas. Devuelve `None` si el planificador aún no está inicializado.
///
/// NO usar desde `schedule`/`run`: esas rutas gestionan el lock a mano porque
/// deben soltarlo ANTES de `switch_to` (que no retorna hasta que la tarea vuelve
/// a planificarse), evitando así un interbloqueo.
fn with_sched<R>(f: impl FnOnce(&mut Scheduler) -> R) -> Option<R> {
    interrupts::critical_section(|| {
        let mut guard = SCHED.lock();
        guard.as_mut().map(f)
    })
}

/// Inicializa el planificador y crea la tarea idle. Idempotente. [CANÓNICO]
/// Llamar antes de `spawn`.
pub fn init() {
    interrupts::critical_section(|| {
        let mut guard = SCHED.lock();
        if guard.is_some() {
            return;
        }
        let mut sched = Scheduler {
            tasks: BTreeMap::new(),
            ready: Vec::new(),
            current: IDLE_TID,
            idle: IDLE_TID,
            next_tid: IDLE_TID + 1,
            slice_remaining: QUANTUM_TICKS,
        };
        // La tarea idle usa el mismo trampolín que el resto; su función de
        // entrada (`idle_entry`) nunca retorna. Si su asignación falla, el
        // scheduler queda sin fallback: se deja registrado pero no se panica.
        if let Ok(idle) = Task::new(IDLE_TID, "idle", idle_entry, 0, layout::DEFAULT_STACK_SIZE, 0)
        {
            sched.tasks.insert(IDLE_TID, idle);
        }
        *guard = Some(sched);
    });
}

/// Crea una tarea nueva y la encola como Ready. Devuelve su Tid. [CANÓNICO]
/// `entry` recibe `arg`. La pila (`stack_size` bytes, 0 = valor por defecto) se
/// asigna en el heap.
pub fn spawn(
    name: &str,
    entry: fn(usize),
    arg: usize,
    stack_size: usize,
    priority: u8,
) -> KResult<Tid> {
    // 1. Reservar un tid nuevo bajo el lock (todavía sin asignar memoria).
    let reserved = with_sched(|s| match s.next_tid.checked_add(1) {
        Some(next) => {
            let tid = s.next_tid;
            s.next_tid = next;
            Ok(tid)
        }
        None => Err(KError::TableFull),
    });
    let tid = match reserved {
        Some(Ok(tid)) => tid,
        Some(Err(e)) => return Err(e),
        None => return Err(KError::NotSupported), // scheduler no inicializado
    };

    // 2. Construir la Task (asigna pila + prepara contexto) FUERA del lock, para
    //    no mantener el estado global bloqueado durante la asignación de memoria.
    let task = Task::new(tid, name, entry, arg, stack_size, priority)?;

    // 3. Insertarla y encolarla como lista.
    let inserted = with_sched(|s| {
        s.tasks.insert(tid, task);
        s.ready.push(tid);
    });
    match inserted {
        Some(()) => Ok(tid),
        // Si el scheduler desapareció entre medias, `task` ya se soltó (y su
        // pila se liberó) al no ejecutarse el cierre.
        None => Err(KError::NotSupported),
    }
}

/// Cede la CPU a la siguiente tarea Ready (cooperativo). [CANÓNICO]
pub fn yield_now() {
    schedule();
}

/// Termina la tarea actual con `code`; la marca Zombie y no retorna. [CANÓNICO]
pub fn exit(code: i32) -> ! {
    with_sched(|s| {
        let cur = s.current;
        if let Some(t) = s.tasks.get_mut(&cur) {
            t.state = TaskState::Zombie;
            t.exit_code = code;
        }
        // Asegurar que no quede en la cola de listas.
        s.ready.retain(|x| *x != cur);
    });
    // Cambiar a otra tarea; para un zombie, `schedule` NO regresa aquí.
    schedule();
    // Red de seguridad: nunca debemos "volver" a una tarea terminada.
    loop {
        core::hint::spin_loop();
    }
}

/// Se llama desde la ISR del tick del timer: contabiliza el quantum y decide
/// preempción. [CANÓNICO]
pub fn tick() {
    let expired = with_sched(|s| {
        if s.slice_remaining > 0 {
            s.slice_remaining -= 1;
        }
        s.slice_remaining == 0
    })
    .unwrap_or(false);

    if expired {
        // Preempción: reprograma desde el contexto de la ISR del tick.
        //
        // RIESGO (hardware): invocar `switch_to` desde una ISR exige que la capa
        // `arch` gestione correctamente el marco de excepción/ventanas al salvar
        // y restaurar contextos. Si esa capa aún no es segura desde ISR, una
        // alternativa es que `tick` marque un flag "need_resched" y que el
        // epílogo del vector llame a `schedule`. Aquí se hace la reprogramación
        // directa (mejor esfuerzo para Fase 2/3).
        schedule();
    }
}

/// Tid de la tarea en ejecución. [CANÓNICO]
pub fn current() -> Tid {
    with_sched(|s| s.current).unwrap_or(IDLE_TID)
}

/// Arranca el bucle del planificador realizando el PRIMER cambio de contexto.
/// No retorna. [CANÓNICO]
pub fn run() -> ! {
    // Deshabilitar interrupciones para el primer cambio de contexto.
    let _prev = interrupts::disable();

    // Contexto desechable del arranque: `switch_to` guardará aquí el estado
    // actual, que NUNCA se restaura porque no volvemos a este punto.
    let mut bootstrap = Context::default();
    let mut target: Option<*const Context> = None;
    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            let first = policy::next_ready(s).unwrap_or(s.idle);
            if let Some(t) = s.tasks.get_mut(&first) {
                t.state = TaskState::Running;
            }
            s.current = first;
            s.slice_remaining = QUANTUM_TICKS;
            target = s.ctx_ptr(first);
        }
    }

    if let Some(next) = target {
        // SAFETY: `next` apunta al `Context` (estable, en `Box`) de la primera
        // tarea; `bootstrap` es válido durante toda la llamada. El lock ya se
        // soltó. No volvemos aquí, así que dejar las interrupciones tal cual las
        // deje la tarea (su PS controla su propio nivel) es correcto.
        unsafe {
            context::switch_to(&mut bootstrap as *mut Context, next);
        }
    }

    // No se alcanza en operación normal (no había ni idle).
    loop {
        core::hint::spin_loop();
    }
}

/// Núcleo de la reprogramación: elige la siguiente tarea y realiza el cambio de
/// contexto. Corre con interrupciones enmascaradas y SUELTA el lock antes de
/// `switch_to` (que no retorna hasta que la tarea vuelve a planificarse).
fn schedule() {
    let prev = interrupts::disable();
    let mut switch: Option<(*mut Context, *const Context)> = None;
    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            let cur = s.current;

            // Recolectar zombies previos (nunca la actual, que aún usa su pila).
            s.reap_zombies_except(cur);

            // Reencolar la tarea actual si sigue siendo ejecutable.
            let still_running = s
                .tasks
                .get(&cur)
                .map(|t| t.state == TaskState::Running)
                .unwrap_or(false);
            if still_running {
                if let Some(t) = s.tasks.get_mut(&cur) {
                    t.state = TaskState::Ready;
                }
                // idle nunca va a la cola: es solo el último recurso.
                if cur != s.idle {
                    s.ready.push(cur);
                }
            }

            // Elegir la siguiente (round-robin); idle si no hay nada listo.
            let next = policy::next_ready(s).unwrap_or(s.idle);
            if let Some(t) = s.tasks.get_mut(&next) {
                t.state = TaskState::Running;
            }
            s.current = next;
            s.slice_remaining = QUANTUM_TICKS;

            if next != cur {
                let cur_ptr = s.ctx_ptr_mut(cur);
                let next_ptr = s.ctx_ptr(next);
                if let (Some(c), Some(n)) = (cur_ptr, next_ptr) {
                    switch = Some((c, n));
                }
            }
        }
    } // <- se suelta el guard del `Mutex` aquí (antes del switch).

    if let Some((cur_ptr, next_ptr)) = switch {
        // SAFETY: ambos punteros refieren a `Context` almacenados en `Box`
        // dentro de la tabla de tareas; no se mueven ni liberan mientras dure el
        // cambio (single-core + interrupciones enmascaradas, y ninguno de los dos
        // se reapea aquí). Se soltó el lock a propósito para no interbloquear,
        // pues `switch_to` no retorna hasta que esta tarea vuelva a correr.
        unsafe {
            context::switch_to(cur_ptr, next_ptr);
        }
    }

    // Al reanudarse esta tarea, se restaura el estado de interrupciones que tenía
    // al entrar en `schedule`.
    interrupts::restore(prev);
}

/// Función de la tarea idle: cede la CPU en bucle para que cualquier tarea lista
/// se ejecute. Nunca retorna (por eso el trampolín jamás llega a `exit`).
fn idle_entry(_arg: usize) {
    loop {
        core::hint::spin_loop();
        yield_now();
    }
}

/// Trampolín común de arranque de TODA tarea. `arg` = tid de la propia tarea.
///
/// El primer `switch_to` hacia una tarea nueva aterriza aquí (así lo prepara
/// `context::init_task_stack`). Recupera la función de entrada real y su
/// argumento, los ejecuta y, al terminar, convierte la tarea en zombie con
/// `exit`. Firma `fn(usize)` para encajar en `init_task_stack`.
///
/// NOTA (hardware): se asume que `init_task_stack` deja el PS de la tarea con
/// las interrupciones habilitadas (PS.INTLEVEL = 0), de modo que las tareas
/// nuevas corran con preempción activa. Es responsabilidad de la capa `arch`.
fn task_trampoline(tid: usize) {
    let tid = tid as Tid;
    let start = with_sched(|s| s.tasks.get(&tid).map(|t| (t.start_entry, t.start_arg))).flatten();
    if let Some((entry, arg)) = start {
        entry(arg);
    }
    exit(0);
}
