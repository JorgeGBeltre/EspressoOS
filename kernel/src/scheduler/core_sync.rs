#![allow(dead_code)]

//! SMP — arranque del segundo núcleo (Fase 9, feature `smp`).
//!
//! Bring-up real de doble núcleo: arranca el APP_CPU (core 1) vía
//! `esp_hal::cpu_control::CpuControl` y lo pone a ejecutar un bucle propio
//! (heartbeat + contador compartido) que demuestra que **ambos LX7 ejecutan
//! nuestro código** y comparten memoria de forma coherente.
//!
//! ## Alcance y siguiente paso
//! Este primer hito NO reescribe el planificador mono-núcleo (validado en
//! hardware) para repartir tareas entre núcleos: eso exige `current`/`idle` por
//! núcleo, disciplina de lock multicore y un `switch_to` verificado en ambos
//! núcleos, y debe validarse en placa. El diseño está en
//! `docs/design/remaining-phases.md`. Aquí el core 1 corre un workload dedicado.
//!
//! El `Mutex` de `arch::xtensa::sync` (IRQ-off + spinlock CAS) YA es seguro entre
//! núcleos para exclusión mutua; lo que falta es el estado por-núcleo del
//! planificador.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static SMP_RUNNING: AtomicBool = AtomicBool::new(false);
// Xtensa LX7 es 32-bit sin atómicos de 64 bits: usamos AtomicU32.
static CORE1_TICKS: AtomicU32 = AtomicU32::new(0);

/// ¿Está el APP_CPU en marcha? (siempre `false` si se compiló sin `smp`).
pub fn is_running() -> bool {
    SMP_RUNNING.load(Ordering::Relaxed)
}

/// Contador de "vueltas" del núcleo 1 (0 sin `smp`). Prueba memoria compartida.
pub fn core1_ticks() -> u64 {
    CORE1_TICKS.load(Ordering::Relaxed) as u64
}

/// Id del núcleo que ejecuta esta llamada (0 = PRO_CPU, 1 = APP_CPU).
pub fn current_core_id() -> usize {
    match esp_hal::Cpu::current() {
        esp_hal::Cpu::ProCpu => 0,
        esp_hal::Cpu::AppCpu => 1,
    }
}

#[cfg(feature = "smp")]
mod imp {
    use super::*;
    use esp_hal::cpu_control::{CpuControl, Stack};
    use esp_hal::peripherals::CPU_CTRL;
    use esp_println::println;

    const APP_STACK_SIZE: usize = 8 * 1024;
    static mut APP_CORE_STACK: Stack<APP_STACK_SIZE> = Stack::new();

    pub fn start(cpu_ctrl: CPU_CTRL) {
        let mut cpu_control = CpuControl::new(cpu_ctrl);
        // SAFETY: `APP_CORE_STACK` sólo se usa aquí y su propiedad pasa al core 1.
        let stack: &'static mut Stack<APP_STACK_SIZE> =
            unsafe { &mut *core::ptr::addr_of_mut!(APP_CORE_STACK) };

        match cpu_control.start_app_core(stack, app_core_main) {
            Ok(guard) => {
                // El guard aparca el núcleo al soltarse: lo mantenemos vivo.
                core::mem::forget(guard);
                SMP_RUNNING.store(true, Ordering::Release);
                println!("[smp] APP_CPU (core 1) started");
            }
            Err(_) => println!("[smp] ERROR: failed to start the APP_CPU"),
        }
    }

    /// Punto de entrada del núcleo 1: entra a su planificador y ejecuta las
    /// tareas encoladas en `ready1` (p. ej. `worker_entry`). No retorna.
    fn app_core_main() {
        crate::scheduler::run_secondary();
    }
}

/// Tarea de demostración que corre EN el núcleo 1 (encolada con `spawn_core1`).
/// Incrementa el contador compartido e imprime su núcleo: si aparece `core1`,
/// prueba que el planificador reparte tareas al segundo núcleo.
#[cfg(feature = "smp")]
pub fn worker_entry(_arg: usize) {
    use esp_println::println;
    let mut last = 0u64;
    loop {
        let now = crate::arch::xtensa::timer::uptime_ms();
        if now.wrapping_sub(last) >= 1000 {
            last = now;
            let t = CORE1_TICKS.fetch_add(1, Ordering::AcqRel) + 1;
            println!(
                "[smp] worker task on core{} tick={} uptime={}ms",
                current_core_id(),
                t,
                now
            );
        }
        crate::scheduler::yield_now();
    }
}

/// Arranca el APP_CPU (core 1). Sólo compila con `--features smp`; `main` la
/// invoca bajo el mismo `cfg`.
#[cfg(feature = "smp")]
pub fn start_secondary_core(cpu_ctrl: esp_hal::peripherals::CPU_CTRL) {
    imp::start(cpu_ctrl);
}
