//! Temporizador del sistema / systick del scheduler (Fase 3).
//!
//! Usa el periférico **SYSTIMER** del ESP32-S3 para dos cosas:
//!   1. Generar una alarma PERIÓDICA a `TICK_HZ`, cuya ISR llama al planificador
//!      (`scheduler::tick`) para contabilizar el quantum y decidir la preempción.
//!   2. Alimentar el reloj monotónico (`uptime_ms`) leyendo el contador.
//!
//! Esta es la ÚNICA capa del kernel que conoce la API (volátil) de timers de
//! `esp-hal`: el resto del kernel solo ve `TICK_HZ` y `uptime_ms()`. Si la API
//! de `esp-hal` cambia entre versiones, SOLO se toca este archivo (contrato §1.4).
//!
//! El SYSTIMER del S3 es un contador de 52 bits que corre a 16 MHz y dispone de
//! tres comparadores (COMP0..COMP2), cada uno capaz de generar interrupción.
//! Aquí usamos un comparador en modo periódico (auto-reload) como systick.
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use core::sync::atomic::{AtomicUsize, Ordering};

use esp_hal::handler; // macro de atributo #[handler] para ISRs compatibles con el HAL
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::timer::{AnyTimer, PeriodicTimer};
use esp_hal::Blocking; // modo del driver: `PeriodicTimer<'d, Dm>` con Dm = Blocking

use super::sync::Mutex;

/// Frecuencia del tick del scheduler (Hz). NO cambiar sin avisar al scheduler:
/// su quantum (`QUANTUM_TICKS`) está calibrado contra este valor. [CANÓNICO]
pub const TICK_HZ: u32 = 100;

/// Tipo del temporizador periódico ya "erosionado" a `AnyTimer` para poder
/// nombrarlo en un `static`. Se usa `AnyTimer` (borra si es SYSTIMER-alarm o
/// TIMG) porque el tipo concreto del alarm de `esp-hal` 0.23 tiene genéricos
/// verbosos y volátiles.
type SchedTimer = PeriodicTimer<'static, Blocking>;

/// Temporizador periódico global.
///
/// INVARIANTE DE CONCURRENCIA (evita interbloqueo con la ISR): el hilo principal
/// SOLO toca este `Mutex` en `init()`, una vez, ANTES de habilitar la
/// interrupción del tick. A partir de ahí únicamente lo toca la ISR
/// (`systimer_tick_isr`) para limpiar el flag. Por tanto nunca hay contención
/// real y `lock()` (que es un spinlock) no puede bloquearse contra sí mismo.
static PERIODIC: Mutex<Option<SchedTimer>> = Mutex::new(None);

/// Gancho de tick registrable. Guarda un puntero a `fn()` (0 = sin registrar).
/// Permite desacoplar la ISR del scheduler (útil para pruebas o para insertar
/// contabilidad extra). Por defecto, si no hay gancho, se llama a
/// `scheduler::tick()`.
static TICK_HANDLER: AtomicUsize = AtomicUsize::new(0);

/// Registra el `handler` que la ISR del systick invocará en cada tick.
/// Debe llamarse en el arranque, antes de `init()` (o justo después), desde el
/// hilo principal. Si nunca se llama, el tick va directo a `scheduler::tick()`.
pub fn set_tick_handler(handler: fn()) {
    TICK_HANDLER.store(handler as usize, Ordering::SeqCst);
}

/// Invoca el gancho registrado, o el tick del scheduler por defecto.
#[inline(always)]
fn invoke_tick_handler() {
    let ptr = TICK_HANDLER.load(Ordering::SeqCst);
    if ptr != 0 {
        // SAFETY: `ptr` solo se escribe en `set_tick_handler` a partir de un
        // `fn()` válido; ambos son del mismo tamaño (puntero) y ABI.
        let f: fn() = unsafe { core::mem::transmute::<usize, fn()>(ptr) };
        f();
    } else {
        // Camino por defecto (contrato §3.2.3): tick del planificador.
        crate::scheduler::tick();
    }
}

/// ISR de la alarma periódica del SYSTIMER.
///
/// Orden IMPRESCINDIBLE:
///   1. Limpiar el flag de la alarma ANTES de nada. Si se hiciera después y
///      `scheduler::tick()` provocara un cambio de contexto (que no retorna
///      hasta que esta tarea vuelva a planificarse), el flag quedaría activo y
///      la interrupción se re-dispararía en bucle.
///   2. Notificar el tick (gancho -> scheduler).
///
/// RIESGO (hardware): que `scheduler::tick()` acabe llamando a `switch_to` desde
/// dentro de esta ISR exige que la capa `arch::context` gestione bien el marco
/// de excepción y las ventanas al conmutar. Si eso aún no fuese seguro desde
/// ISR, la alternativa es que `tick()` solo marque un flag "need_resched" y que
/// la reprogramación se haga en el epílogo del vector. Esa decisión vive en el
/// scheduler; aquí solo se le cede el control.
#[handler]
fn systimer_tick_isr() {
    // (1) Limpiar el flag de la alarma para no re-entrar.
    //
    // No hay contención con el hilo principal (ver INVARIANTE en `PERIODIC`), así
    // que `lock()` es inmediato aquí dentro de la ISR.
    if let Some(t) = PERIODIC.lock().as_mut() {
        t.clear_interrupt(); // (?) método del trait `Timer`/`PeriodicTimer` en 0.23
    }

    // (2) Ceder el control al tick del kernel.
    invoke_tick_handler();
}

/// Arranca el systick a `TICK_HZ`; su ISR llama a `scheduler::tick()`. Fase 3.
/// [CANÓNICO]
///
/// Firma pública SIN periférico (contrato §3.2.3): como `main` ya consumió el
/// struct `Peripherals`, aquí se recupera el SYSTIMER con `steal()`. Es correcto
/// porque se llama UNA sola vez, en el arranque, y es el único dueño del
/// SYSTIMER en el kernel. Idempotente: si ya se inicializó, no hace nada.
pub fn init() {
    // Idempotencia: no reprogramar si ya hay un periódico vivo.
    if PERIODIC.lock().is_some() {
        return;
    }

    // --- Zona dependiente de la API de esp-hal 0.23 (marcada (?), a verificar) ---
    //
    // `steal()` obtiene el singleton del periférico sin que main nos lo pase.
    // SAFETY: se invoca una sola vez en el arranque; nadie más posee el SYSTIMER.
    let systimer_perif = unsafe { esp_hal::peripherals::SYSTIMER::steal() }; // (?) steal()
    let systimer = SystemTimer::new(systimer_perif);

    // Tomamos la alarma 0 y la erosionamos a `AnyTimer` para el `static`.
    // (?) nombre del campo `.alarm0` y `From<Alarm> for AnyTimer` en 0.23.
    let alarm: AnyTimer = systimer.alarm0.into();
    let mut periodic = PeriodicTimer::new(alarm);

    // Enganchamos la ISR ANTES de arrancar, para no perder ningún disparo.
    // `#[handler]` hace que `systimer_tick_isr` sea un `InterruptHandler`.
    periodic.set_interrupt_handler(systimer_tick_isr); // (?) firma exacta en 0.23

    // Periodo = 1 s / TICK_HZ. A 100 Hz => 10_000 µs (10 ms).
    // `TICK_HZ` es constante > 0, así que la división es segura.
    let periodo_us: u64 = 1_000_000u64 / TICK_HZ as u64;

    // Arrancar en modo periódico (auto-reload). `start` devuelve Result; en el
    // kernel no hacemos panic: descartamos el Err (mejor un systick ausente que
    // un panic en el arranque; el fallo se notaría por falta de preempción).
    // (?) tipo de duración exacto de `start` en 0.23.
    // `start` toma `fugit::MicrosDurationU64`, que ES exactamente
    // `esp_hal::time::Duration` (fugit Duration a 1 µs). `from_ticks(n)` = n µs.
    let _ = periodic.start(esp_hal::time::Duration::from_ticks(periodo_us));

    // ORDEN IMPORTANTE (cierra la ventana de carrera): guardamos el objeto en
    // `PERIODIC` y SOLO DESPUÉS habilitamos la interrupción, y ambas cosas con el
    // `lock` tomado (IRQs enmascaradas). Así el primer tick no puede llegar hasta
    // soltar el guard, momento en que `PERIODIC` ya es `Some` y la ISR sí podrá
    // limpiar el flag. Guardar el objeto además: (a) mantiene el hardware vivo
    // (no se hace `drop`, que pararía el timer) y (b) da acceso a la ISR para
    // limpiar el flag de alarma.
    {
        let mut g = PERIODIC.lock();
        *g = Some(periodic);
        if let Some(t) = g.as_mut() {
            t.enable_interrupt(true); // (?) método en 0.23
        }
    }
    // --- Fin zona dependiente de esp-hal ---
}

/// Milisegundos monotónicos desde el arranque. [CANÓNICO]
///
/// Camino recomendado (contrato §1.4): reloj monotónico del HAL, que en el S3 se
/// apoya en el propio SYSTIMER. No panica: si algo fuese inválido, devuelve 0.
///
/// FALLBACK (si los nombres `now()/duration_since_epoch()/to_millis()` no
/// cuadran con la versión instalada): leer directamente el contador del SYSTIMER
/// (16 MHz => 16_000 ticks/ms) mediante `SystemTimer::unit_value(Unit::Unit0)` o
/// el registro crudo, y dividir por 16_000. Absorber ese cambio SOLO aquí.
pub fn uptime_ms() -> u64 {
    // (?) nombres exactos de la cadena de métodos en esp-hal 0.23.
    esp_hal::time::now().duration_since_epoch().to_millis()
}
