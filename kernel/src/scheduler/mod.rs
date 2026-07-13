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

const QUANTUM_TICKS: u32 = 5;

const IDLE_TID: Tid = 0;

struct Scheduler {

    tasks: BTreeMap<Tid, Box<Task>>,

    ready: Vec<Tid>,

    current: Tid,

    idle: Tid,

    next_tid: Tid,

    slice_remaining: u32,

    // --- Estado del núcleo 1 (SMP). Sólo existe con la feature `smp`; el camino
    // mono-núcleo por defecto queda intacto. El core 1 usa su PROPIA cola
    // (`ready1`) para no arrastrar la tarea de WiFi (esp-wifi tiene afinidad al
    // core 0); `tasks` y el lock sí son compartidos. ---
    #[cfg(feature = "smp")]
    current1: Tid,

    #[cfg(feature = "smp")]
    idle1: Tid,

    #[cfg(feature = "smp")]
    ready1: Vec<Tid>,
}

impl Scheduler {

    fn ctx_ptr(&self, tid: Tid) -> Option<*const Context> {
        self.tasks.get(&tid).map(|t| &t.context as *const Context)
    }

    fn ctx_ptr_mut(&mut self, tid: Tid) -> Option<*mut Context> {
        self.tasks
            .get_mut(&tid)
            .map(|t| &mut t.context as *mut Context)
    }

    fn reap_zombies_except(&mut self, keep: Tid) {
        let mut dead: Vec<Tid> = Vec::new();
        for (tid, t) in self.tasks.iter() {
            if t.state == TaskState::Zombie && *tid != keep {
                // Nunca reciclar una tarea que sigue siendo `current` en el otro
                // núcleo (ventana durante su exit antes del cambio de contexto).
                #[cfg(feature = "smp")]
                {
                    if *tid == self.current || *tid == self.current1 {
                        continue;
                    }
                }
                dead.push(*tid);
            }
        }
        for tid in dead {
            self.ready.retain(|x| *x != tid);

            self.tasks.remove(&tid);
        }
    }
}

static SCHED: Mutex<Option<Scheduler>> = Mutex::new(None);

fn with_sched<R>(f: impl FnOnce(&mut Scheduler) -> R) -> Option<R> {
    interrupts::critical_section(|| {
        let mut guard = SCHED.lock();
        guard.as_mut().map(f)
    })
}

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
            #[cfg(feature = "smp")]
            current1: IDLE_TID,
            #[cfg(feature = "smp")]
            idle1: IDLE_TID,
            #[cfg(feature = "smp")]
            ready1: Vec::new(),
        };

        if let Ok(idle) = Task::new(IDLE_TID, "idle", idle_entry, 0, layout::DEFAULT_STACK_SIZE, 0)
        {
            sched.tasks.insert(IDLE_TID, idle);
        }

        // Bajo SMP, el núcleo 1 tiene su propia tarea idle.
        #[cfg(feature = "smp")]
        {
            let idle1_tid = sched.next_tid;
            sched.next_tid += 1;
            if let Ok(idle1) =
                Task::new(idle1_tid, "idle1", idle_entry, 0, layout::DEFAULT_STACK_SIZE, 0)
            {
                sched.tasks.insert(idle1_tid, idle1);
            }
            sched.idle1 = idle1_tid;
            sched.current1 = idle1_tid;
        }

        *guard = Some(sched);
    });
}

pub fn spawn(
    name: &str,
    entry: fn(usize),
    arg: usize,
    stack_size: usize,
    priority: u8,
) -> KResult<Tid> {

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
        None => return Err(KError::NotSupported),
    };

    let task = Task::new(tid, name, entry, arg, stack_size, priority)?;

    let inserted = with_sched(|s| {
        s.tasks.insert(tid, task);
        s.ready.push(tid);
    });
    match inserted {
        Some(()) => Ok(tid),

        None => Err(KError::NotSupported),
    }
}

pub fn yield_now() {
    schedule();
}

pub fn exit(code: i32) -> ! {
    with_sched(|s| {
        #[cfg(feature = "smp")]
        let cur = if core_id() == 1 { s.current1 } else { s.current };
        #[cfg(not(feature = "smp"))]
        let cur = s.current;
        if let Some(t) = s.tasks.get_mut(&cur) {
            t.state = TaskState::Zombie;
            t.exit_code = code;
        }

        s.ready.retain(|x| *x != cur);
        #[cfg(feature = "smp")]
        s.ready1.retain(|x| *x != cur);
    });

    schedule();

    loop {
        core::hint::spin_loop();
    }
}

pub fn tick() {
    let expired = with_sched(|s| {
        if s.slice_remaining > 0 {
            s.slice_remaining -= 1;
        }
        s.slice_remaining == 0
    })
    .unwrap_or(false);

    if expired {

        schedule();
    }
}

pub fn current() -> Tid {
    #[cfg(feature = "smp")]
    let cur = {
        let c = core_id();
        with_sched(|s| if c == 1 { s.current1 } else { s.current })
    };
    #[cfg(not(feature = "smp"))]
    let cur = with_sched(|s| s.current);
    cur.unwrap_or(IDLE_TID)
}

pub fn run() -> ! {

    let _prev = interrupts::disable();

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

        unsafe {
            context::switch_to(&mut bootstrap as *mut Context, next);
        }
    }

    loop {
        core::hint::spin_loop();
    }
}

fn schedule() {
    let prev = interrupts::disable();

    // Bajo SMP, el núcleo 1 planifica desde su propia cola.
    #[cfg(feature = "smp")]
    if core_id() == 1 {
        schedule_core1();
        interrupts::restore(prev);
        return;
    }

    let mut switch: Option<(*mut Context, *const Context)> = None;
    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            let cur = s.current;

            s.reap_zombies_except(cur);

            let still_running = s
                .tasks
                .get(&cur)
                .map(|t| t.state == TaskState::Running)
                .unwrap_or(false);
            if still_running {
                if let Some(t) = s.tasks.get_mut(&cur) {
                    t.state = TaskState::Ready;
                }

                if cur != s.idle {
                    s.ready.push(cur);
                }
            }

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
    }

    if let Some((cur_ptr, next_ptr)) = switch {

        unsafe {
            context::switch_to(cur_ptr, next_ptr);
        }
    }

    interrupts::restore(prev);
}

fn idle_entry(_arg: usize) {
    loop {
        core::hint::spin_loop();
        yield_now();
    }
}

fn task_trampoline(tid: usize) {
    let tid = tid as Tid;
    let start = with_sched(|s| s.tasks.get(&tid).map(|t| (t.start_entry, t.start_arg))).flatten();
    if let Some((entry, arg)) = start {
        entry(arg);
    }
    exit(0);
}

// ===========================================================================
// SMP (Fase 9, feature `smp`): planificación en el núcleo 1 desde su cola propia.
// ===========================================================================

/// Id del núcleo que ejecuta esta llamada (0 = PRO_CPU, 1 = APP_CPU).
#[cfg(feature = "smp")]
#[inline]
fn core_id() -> usize {
    match esp_hal::Cpu::current() {
        esp_hal::Cpu::AppCpu => 1,
        _ => 0,
    }
}

/// Crea una tarea que se ejecutará en el **núcleo 1** (cola `ready1`).
#[cfg(feature = "smp")]
pub fn spawn_core1(
    name: &str,
    entry: fn(usize),
    arg: usize,
    stack_size: usize,
    priority: u8,
) -> KResult<Tid> {
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
        None => return Err(KError::NotSupported),
    };
    let task = Task::new(tid, name, entry, arg, stack_size, priority)?;
    let inserted = with_sched(|s| {
        s.tasks.insert(tid, task);
        s.ready1.push(tid);
    });
    match inserted {
        Some(()) => Ok(tid),
        None => Err(KError::NotSupported),
    }
}

/// Planificador del núcleo 1 (llamado desde `schedule()` cuando `core_id()==1`).
/// Interrupciones ya deshabilitadas por el llamador.
#[cfg(feature = "smp")]
fn schedule_core1() {
    let mut switch: Option<(*mut Context, *const Context)> = None;
    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            let cur = s.current1;

            s.reap_zombies_except(cur);

            let still_running = s
                .tasks
                .get(&cur)
                .map(|t| t.state == TaskState::Running)
                .unwrap_or(false);
            if still_running {
                if let Some(t) = s.tasks.get_mut(&cur) {
                    t.state = TaskState::Ready;
                }
                if cur != s.idle1 {
                    s.ready1.push(cur);
                }
            }

            let next = if s.ready1.is_empty() {
                s.idle1
            } else {
                s.ready1.remove(0)
            };
            if let Some(t) = s.tasks.get_mut(&next) {
                t.state = TaskState::Running;
            }
            s.current1 = next;

            if next != cur {
                let cur_ptr = s.ctx_ptr_mut(cur);
                let next_ptr = s.ctx_ptr(next);
                if let (Some(c), Some(n)) = (cur_ptr, next_ptr) {
                    switch = Some((c, n));
                }
            }
        }
    }

    if let Some((cur_ptr, next_ptr)) = switch {
        unsafe {
            context::switch_to(cur_ptr, next_ptr);
        }
    }
}

/// Punto de entrada del núcleo 1: arranca el planificador de su cola. No retorna.
#[cfg(feature = "smp")]
pub fn run_secondary() -> ! {
    let _prev = interrupts::disable();

    let mut bootstrap = Context::default();
    let mut target: Option<*const Context> = None;
    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            let first = if s.ready1.is_empty() {
                s.idle1
            } else {
                s.ready1.remove(0)
            };
            if let Some(t) = s.tasks.get_mut(&first) {
                t.state = TaskState::Running;
            }
            s.current1 = first;
            target = s.ctx_ptr(first);
        }
    }

    if let Some(next) = target {
        unsafe {
            context::switch_to(&mut bootstrap as *mut Context, next);
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
