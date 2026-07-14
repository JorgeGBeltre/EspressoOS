#![allow(dead_code)]

use alloc::collections::BTreeMap;

use crate::arch::xtensa::context::{self, Context};
use crate::arch::xtensa::interrupts;
use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

pub mod core_sync;
pub mod policy;
pub mod task;
pub mod process;

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

    // --- Estado del núcleo 1 (SMP). Sólo existe con la feature `smp`. ---
    #[cfg(feature = "smp")]
    current1: Tid,

    #[cfg(feature = "smp")]
    idle1: Tid,
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

use core::sync::atomic::{AtomicBool, Ordering};

static NEED_RESCHED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

/// Señala que el syscall en curso quiere **re-ejecutarse** al reanudar la tarea
/// (semántica bloqueante, p.ej. `wait`). El epílogo del trap la consulta: si está
/// puesta, NO avanza el PC ni sobreescribe `A2`, dejando la instrucción `syscall`
/// intacta para que se vuelva a ejecutar cuando la tarea sea replanificada.
static RESTART_SYSCALL: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

pub fn set_restart_syscall() {
    let core = core_id();
    RESTART_SYSCALL[core].store(true, Ordering::Relaxed);
}

/// Devuelve y limpia la bandera de reinicio del syscall para el núcleo actual.
pub fn take_restart_syscall() -> bool {
    let core = core_id();
    RESTART_SYSCALL[core].swap(false, Ordering::Relaxed)
}

pub fn set_need_resched() {
    let core = core_id();
    NEED_RESCHED[core].store(true, Ordering::Relaxed);
}

pub fn set_need_resched_core(core: usize) {
    if core < 2 {
        NEED_RESCHED[core].store(true, Ordering::Relaxed);
    }
}

pub fn clear_need_resched() {
    let core = core_id();
    NEED_RESCHED[core].store(false, Ordering::Relaxed);
}

pub fn need_resched() -> bool {
    let core = core_id();
    NEED_RESCHED[core].load(Ordering::Relaxed)
}

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
        };

        if let Ok(idle) = Task::new(IDLE_TID, "idle", idle_entry, 0, layout::DEFAULT_STACK_SIZE, 0, false)
        {
            let mut t = idle;
            t.affinity = Some(0);
            sched.tasks.insert(IDLE_TID, t);
        }

        // Bajo SMP, el núcleo 1 tiene su propia tarea idle.
        #[cfg(feature = "smp")]
        {
            let idle1_tid = sched.next_tid;
            sched.next_tid += 1;
            if let Ok(idle1) =
                Task::new(idle1_tid, "idle1", idle_entry, 0, layout::DEFAULT_STACK_SIZE, 0, false)
            {
                let mut t = idle1;
                t.affinity = Some(1);
                sched.tasks.insert(idle1_tid, t);
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
    is_user: bool,
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

    let mut task = Task::new(tid, name, entry, arg, stack_size, priority, is_user)?;
    if name == "net" {
        task.affinity = Some(0);
    }

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
    crate::syscall::invoke(crate::syscall::Syscall::Yield as usize, [0; crate::syscall::MAX_ARGS]);
}

pub fn exit(code: i32) -> ! {
    mark_zombie(code);

    set_need_resched();

    // Invocar el syscall de exit para forzar el trap de replanificación y no volver
    crate::syscall::invoke(crate::syscall::Syscall::Exit as usize, [code as usize; crate::syscall::MAX_ARGS]);

    loop {
        core::hint::spin_loop();
    }
}

pub fn mark_zombie(code: i32) {
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
    });
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
        set_need_resched();
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
    let mut target: Option<(u32, bool, u32)> = None; // (frame_ptr, is_user, sp)
    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            let first = policy::next_ready(s, 0).unwrap_or(s.idle);
            if let Some(t) = s.tasks.get_mut(&first) {
                t.state = TaskState::Running;
            }
            s.current = first;
            s.slice_remaining = QUANTUM_TICKS;
            let task = s.tasks.get(&first).unwrap();
            let base = task.stack_base as u32;
            let top = (task.stack_base as usize + task.stack_size) as u32;
            crate::mm::mpu::configure_stack_guard(0, base, top);
            // El frame vive dentro del Task (Box estable en SCHED): su dirección
            // sigue válida tras soltar el lock.
            let fp = &task.context.frame as *const _ as u32;
            target = Some((fp, task.is_user, task.context.frame.A1));
        }
    }

    if let Some((frame_ptr, is_user, sp)) = target {
        crate::mm::mpu::prepare_world_switch(is_user, sp);
        unsafe {
            context::resume_task(frame_ptr);
        }
    }

    loop {
        core::hint::spin_loop();
    }
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

#[inline(always)]
fn core_id() -> usize {
    #[cfg(feature = "smp")]
    {
        match esp_hal::Cpu::current() {
            esp_hal::Cpu::AppCpu => 1,
            _ => 0,
        }
    }
    #[cfg(not(feature = "smp"))]
    {
        0
    }
}

pub fn preempt_switch(save_frame: &mut esp_hal::xtensa_lx_rt::exception::Context) {
    clear_need_resched();

    let prev = interrupts::disable();
    let core = core_id();

    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            #[cfg(feature = "smp")]
            let cur = if core == 1 { s.current1 } else { s.current };
            #[cfg(not(feature = "smp"))]
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
                if core == 1 {
                    #[cfg(feature = "smp")]
                    {
                        if cur != s.idle1 {
                            s.ready.push(cur);
                        }
                    }
                } else {
                    if cur != s.idle {
                        s.ready.push(cur);
                    }
                }
            }

            // Guardar el contexto de la tarea saliente: copiar los registros vivos
            // FUERA del frame del vector, a su almacenamiento por-tarea.
            if let Some(t) = s.tasks.get_mut(&cur) {
                t.context.frame = *save_frame;
            }

            // Elegir la siguiente tarea
            #[cfg(feature = "smp")]
            let next = if core == 1 {
                policy::next_ready(s, 1).unwrap_or(s.idle1)
            } else {
                policy::next_ready(s, 0).unwrap_or(s.idle)
            };
            #[cfg(not(feature = "smp"))]
            let next = policy::next_ready(s, 0).unwrap_or(s.idle);

            if let Some(t) = s.tasks.get_mut(&next) {
                t.state = TaskState::Running;
            }

            #[cfg(feature = "smp")]
            {
                if core == 1 {
                    s.current1 = next;
                } else {
                    s.current = next;
                    s.slice_remaining = QUANTUM_TICKS;
                }
            }
            #[cfg(not(feature = "smp"))]
            {
                s.current = next;
                s.slice_remaining = QUANTUM_TICKS;
            }

            let next_task = s.tasks.get(&next).unwrap();
            let base = next_task.stack_base as u32;
            let top = (next_task.stack_base as usize + next_task.stack_size) as u32;
            crate::mm::mpu::configure_stack_guard(core, base, top);

            if next != cur {
                // Conmutación REAL: copiar el contexto de la siguiente tarea DENTRO
                // de *save_frame. Al volver del handler, el vector de xtensa-lx-rt
                // restaura *save_frame -> registros de la siguiente tarea (su A1
                // cambia de pila, su PC/PS dirigen el rfe). Sin trucos de `a5`.
                *save_frame = next_task.context.frame;
                crate::mm::mpu::prepare_world_switch(
                    next_task.is_user,
                    next_task.context.frame.A1,
                );
            }
        }
    }

    interrupts::restore(prev);
}

// ===========================================================================
// SMP (Fase 9, feature `smp`): planificación en el núcleo 1.
// ===========================================================================

/// Crea una tarea que se ejecutará en el **núcleo 1**.
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
    let mut task = Task::new(tid, name, entry, arg, stack_size, priority, false)?;
    task.affinity = Some(1);
    let inserted = with_sched(|s| {
        s.tasks.insert(tid, task);
        s.ready.push(tid);
    });
    match inserted {
        Some(()) => Ok(tid),
        None => Err(KError::NotSupported),
    }
}

/// Punto de entrada del núcleo 1: arranca el planificador. No retorna.
#[cfg(feature = "smp")]
pub fn run_secondary() -> ! {
    let _prev = interrupts::disable();

    let mut target: Option<u32> = None;
    {
        let mut guard = SCHED.lock();
        if let Some(s) = guard.as_mut() {
            let first = policy::next_ready(s, 1).unwrap_or(s.idle1);
            if let Some(t) = s.tasks.get_mut(&first) {
                t.state = TaskState::Running;
            }
            s.current1 = first;
            let task = s.tasks.get(&first).unwrap();
            let base = task.stack_base as u32;
            let top = (task.stack_base as usize + task.stack_size) as u32;
            crate::mm::mpu::configure_stack_guard(1, base, top);
            target = Some(task.context.sp);
        }
    }

    if let Some(next_sp) = target {
        unsafe {
            context::resume_task(next_sp);
        }
    }

    loop {
        core::hint::spin_loop();
    }
}

pub fn set_task_user(tid: Tid, is_user: bool) {
    with_sched(|s| {
        if let Some(t) = s.tasks.get_mut(&tid) {
            t.is_user = is_user;
        }
    });
}

pub fn block_current() {
    with_sched(|s| {
        #[cfg(feature = "smp")]
        let cur = if core_id() == 1 { s.current1 } else { s.current };
        #[cfg(not(feature = "smp"))]
        let cur = s.current;

        if let Some(t) = s.tasks.get_mut(&cur) {
            t.state = TaskState::Blocked;
        }
        s.ready.retain(|x| *x != cur);
    });
    set_need_resched();
    yield_now();
}

/// Como [`block_current`], pero **no** cede la CPU aquí (no ejecuta `yield_now`,
/// es decir, no emite la instrucción `syscall`). Pensada para llamarse DENTRO de
/// un handler de syscall: el `preempt_switch` del epílogo del trap hará la
/// conmutación con el `save_frame` correcto tras retornar del handler. Llamar
/// `yield_now()` desde dentro del handler sería un `syscall` **anidado** dentro de
/// `__exception`, que corrompe el cambio de contexto (o dispara doble excepción).
pub fn block_current_noswitch() {
    with_sched(|s| {
        #[cfg(feature = "smp")]
        let cur = if core_id() == 1 { s.current1 } else { s.current };
        #[cfg(not(feature = "smp"))]
        let cur = s.current;

        if let Some(t) = s.tasks.get_mut(&cur) {
            t.state = TaskState::Blocked;
        }
        s.ready.retain(|x| *x != cur);
    });
    set_need_resched();
}

pub fn unblock_task(tid: Tid) {
    with_sched(|s| {
        if let Some(t) = s.tasks.get_mut(&tid) {
            if t.state == TaskState::Blocked {
                t.state = TaskState::Ready;
                s.ready.push(tid);
            }
        }
    });
}
