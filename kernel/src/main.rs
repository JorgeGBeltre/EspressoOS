//! esp32s3-os — punto de entrada del kernel.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Orquesta el arranque siguiendo la "Secuencia de arranque" del contrato (§5):
//! inicializa el HAL y el reloj, el heap del kernel, la capa de arquitectura
//! (interrupciones + systick del planificador), monta el VFS (`ramfs` en `/` y
//! `/tmp`, `devfs` en `/dev`), levanta la consola y, finalmente, arranca el
//! planificador con dos tareas: la shell interactiva y un latido (blink) del LED.
//!
//! Piezas cuya complejidad de hardware absorben los subsistemas (no `main`):
//!  - El init del SYSTIMER/tick lo hace `arch::xtensa::timer` (roba el periférico).
//!  - Los drivers de bus (spi/i2c/flash/wifi) y el FS de flash (littlefs) NO se
//!    inicializan aquí: pertenecen a fases posteriores (§5, pasos 11/13/14) y su
//!    bring-up real requiere ceder periféricos concretos. `main` se ciñe al
//!    camino mínimo que deja el sistema interactivo.
#![no_std]
#![no_main]
// `asm!` en Xtensa es una arquitectura "experimental" para inline-asm: requiere
// este feature-gate (el toolchain `esp` es nightly, así que está disponible).
#![feature(asm_experimental_arch)]
#![allow(dead_code, unused_imports, unused_variables)]

extern crate alloc;

// Subsistemas del kernel. `prelude` primero conceptualmente (tipos compartidos);
// declarado en orden alfabético como el resto.
mod arch;
mod drivers;
mod fs;
mod mm;
mod ota;
mod prelude;
mod scheduler;
mod shell;
mod syscall;
mod vfs;
// Credenciales WiFi de desarrollo (archivo NO versionado; ver el `.example`).
// `drivers::wifi` las lee vía `crate::wifi_credentials::{WIFI_SSID, WIFI_PASSWORD}`.
mod wifi_credentials;

use esp_backtrace as _; // instala panic handler + backtrace
use esp_hal::clock::CpuClock;
use esp_hal::main;
use esp_println::println;

use crate::prelude::*;

/// Pin del LED integrado usado como latido de vida. Ajustar según la placa:
/// muchas ESP32-S3-DevKit usan GPIO2 o GPIO48 (LED RGB).
const LED_GPIO: u8 = 2;

/// Prioridad por defecto de las tareas de sistema (0 = más baja). El scheduler
/// de Fase 2 es round-robin, así que la prioridad es informativa por ahora.
const PRIO_DEFAULT: u8 = 1;

/// Periodo del latido del LED, en milisegundos.
const HEARTBEAT_MS: u64 = 500;

/// Tamaño de pila de la tarea de red. Mayor que el por defecto (8 KB) porque el
/// bring-up de esp-wifi + smoltcp (iface, handshakes) usa más pila que las tareas
/// triviales. Los búferes grandes (SocketSet, RX/TX TCP) van al heap, no a la pila.
const NET_STACK_SIZE: usize = 16 * 1024;

#[main]
fn main() -> ! {
    // -- §5.1  Inicialización del HAL a máxima frecuencia de CPU (240 MHz). -----
    // CAMBIO (Fase 1/7): AHORA conservamos los periféricos. `esp-wifi` necesita
    // poseer WIFI, RADIO_CLK, TIMG0 y RNG (se ceden a `drivers::wifi`), y la PSRAM
    // se mapea DENTRO de `esp_hal::init` vía `Config::psram` (esp-hal 0.23 ya no
    // tiene función suelta de init de PSRAM). `CpuClock::max()` (240 MHz) además
    // satisface el requisito de esp-wifi de CPU >= 80 MHz.
    let peripherals = esp_hal::init(
        esp_hal::Config::default()
            .with_cpu_clock(CpuClock::max())
            .with_psram(esp_hal::psram::PsramConfig {
                // PSRAM octal de 8 MB. `AutoDetect` también valdría; la fijamos para
                // no depender de la lectura de densidad.
                size: esp_hal::psram::PsramSize::Size(8 * 1024 * 1024),
                ..Default::default()
            }),
    );

    // -- §5.2  Heap del kernel (SRAM interna). OBLIGATORIO antes de cualquier ---
    // asignación dinámica (Vec/String/Box/Arc).
    mm::heap::init();

    // -- §5.3  PSRAM -> heap secundario (Fase 1). La PSRAM ya está mapeada por ---
    // `esp_hal::init`; obtenemos su rango con `psram_raw_parts` y la registramos en
    // el allocator (`MemoryCapability::External`). Se añade ANTES de arrancar la
    // red para que esp-wifi/smoltcp tengan holgura de heap.
    // NOTA: la región interna (SRAM) se registra primero (arriba), así que las
    // asignaciones pequeñas de esp-wifi tienden a caer en SRAM; la PSRAM absorbe
    // los búferes grandes. (Ver RIESGO de DMA en el manifiesto.)
    let (psram_base, psram_len) = esp_hal::psram::psram_raw_parts(&peripherals.PSRAM);
    mm::heap::add_psram(psram_base, psram_len);
    println!(
        "[kernel] PSRAM añadida al heap: {} bytes @ {:p}",
        psram_len, psram_base
    );

    // -- §5.4  Consola (USB-Serial-JTAG), base de `/dev/console`. --------------
    // No es fatal si falla: el banner de bring-up usa `esp-println`, que funciona
    // aun sin este driver.
    if let Err(e) = drivers::uart::init() {
        println!("[kernel] aviso: drivers::uart::init fallo: {:?}", e);
    }

    // -- §5.5  Banner por consola (bring-up). ----------------------------------
    banner();

    // -- §5.6  Interrupciones: prepara la capa de secciones críticas/vectores. -
    arch::xtensa::interrupts::init();

    // -- §5.8  Protección de memoria (PMS). No-op hasta Fase 8 (seguro). -------
    mm::mpu::init();

    // -- §5.9  VFS: tablas de montajes y descriptores. -------------------------
    if let Err(e) = vfs::init() {
        println!("[kernel] aviso: vfs::init fallo: {:?}", e);
    }

    // -- §5.10 devfs en `/dev` (registra /dev/null, /dev/zero, /dev/console). --
    match vfs::devfs::init() {
        Ok(devfs) => {
            if let Err(e) = vfs::mount("/dev", devfs) {
                println!("[kernel] aviso: mount /dev fallo: {:?}", e);
            }
        }
        Err(e) => println!("[kernel] aviso: devfs::init fallo: {:?}", e),
    }

    // -- §5.11 FS raíz. En esta fase se usa `ramfs` en `/` (el binding de -------
    // littlefs sobre flash es borrador). `/tmp` también es un `ramfs`
    // independiente (§5.12). Ambos son volátiles: se pierden al reiniciar.
    if let Err(e) = vfs::mount("/", fs::ramfs::RamFs::new()) {
        println!("[kernel] aviso: mount / fallo: {:?}", e);
    }
    if let Err(e) = vfs::mount("/tmp", fs::ramfs::RamFs::new()) {
        println!("[kernel] aviso: mount /tmp fallo: {:?}", e);
    }

    // -- §5.15 Planificador: crea la tarea idle. ANTES de `spawn`. -------------
    scheduler::init();

    // -- §5.16 Tareas: shell interactiva + latido del LED. ---------------------
    match scheduler::spawn("shell", shell_task, 0, layout::DEFAULT_STACK_SIZE, PRIO_DEFAULT) {
        Ok(tid) => println!("[kernel] tarea 'shell' creada (tid={})", tid),
        Err(e) => println!("[kernel] ERROR: no se pudo crear la shell: {:?}", e),
    }
    match scheduler::spawn(
        "heartbeat",
        heartbeat_task,
        0,
        layout::DEFAULT_STACK_SIZE,
        PRIO_DEFAULT,
    ) {
        Ok(tid) => println!("[kernel] tarea 'heartbeat' creada (tid={})", tid),
        Err(e) => println!("[kernel] aviso: no se pudo crear el latido: {:?}", e),
    }

    // -- §5.14 Red: cedemos a esp-wifi los periféricos que exige y arrancamos ---
    // la tarea de red. DECISIÓN: como `spawn` solo admite `fn(usize)`, depositamos
    // los periféricos en un `static` de `drivers::wifi` ANTES del spawn; `net_task`
    // los recoge y hace toda la bring-up (init radio + STA + smoltcp + DHCP + eco)
    // dentro de su bucle, cediendo con `scheduler::yield_now()`. La bring-up REAL
    // ocurre DESPUÉS de `scheduler::run()` (con interrupciones activas), que es lo
    // que esp-wifi necesita para progresar su firmware.
    drivers::wifi::provide_peripherals(
        peripherals.TIMG0,
        peripherals.RNG,
        peripherals.RADIO_CLK,
        peripherals.WIFI,
    );
    match scheduler::spawn("net", drivers::wifi::net_task, 0, NET_STACK_SIZE, PRIO_DEFAULT) {
        Ok(tid) => println!("[kernel] tarea 'net' creada (tid={})", tid),
        Err(e) => println!("[kernel] aviso: no se pudo crear la red: {:?}", e),
    }

    // -- §5.7  Systick del scheduler (SYSTIMER @ TICK_HZ -> scheduler::tick). ---
    // DECISIÓN DE INTEGRACIÓN (reordenado respecto a §5.7): el systick se arma
    // JUSTO ANTES de `run()`, con las interrupciones globalmente enmascaradas, para
    // cerrar la ventana de carrera en la que un tick podría reprogramar el
    // planificador antes del primer cambio de contexto. `run()` deja el nivel de
    // interrupciones en manos del PS de la primera tarea (que arranca con
    // preempción activa). Nunca restauramos aquí: `run()` no retorna.
    let _ = arch::xtensa::interrupts::disable();
    arch::xtensa::timer::init();
    println!("[kernel] arrancando el planificador...");

    // -- §5.18 Arrancar el bucle del planificador. NO retorna. -----------------
    scheduler::run();
}

/// Imprime el banner de bring-up por la consola (`esp-println`).
fn banner() {
    println!();
    println!("========================================");
    println!("   esp32s3-os   ·   kernel");
    println!("   Consola viva. Arrancando subsistemas.");
    println!("   Heap del kernel: {} bytes", mm::heap::size());
    println!("========================================");
}

/// Cuerpo de la tarea de la shell: ejecuta el REPL interactivo. `shell::run`
/// no retorna en operación normal; si lo hiciera (salida limpia), la tarea
/// termina y el scheduler la reaparea.
fn shell_task(_arg: usize) {
    shell::run();
}

/// Cuerpo de la tarea de latido: parpadea el LED como señal de vida.
///
/// Usa SOLO API canónica de `drivers::gpio` (`configure` + `write`, §3.9); no
/// emplea `esp_hal::Delay` (prohibido dentro del planificador preemptivo, §1.3),
/// sino una espera cooperativa basada en el reloj monotónico (`uptime_ms`) que
/// cede la CPU. Silenciosa a propósito: no escribe en consola para no interferir
/// con el eco de la shell (ambas comparten el USB-Serial-JTAG).
fn heartbeat_task(_arg: usize) {
    // Configurar el pin del LED como salida. Si falla (pin inválido), la tarea
    // sigue viva cediendo la CPU: nunca panica.
    let _ = drivers::gpio::configure(LED_GPIO, drivers::gpio::PinMode::Output);

    let mut encendido = false;
    let mut tick: u64 = 0;
    loop {
        encendido = !encendido;
        let _ = drivers::gpio::write(LED_GPIO, encendido);
        // DIAGNÓSTICO (P1/P2): imprime por COM5 para confirmar que el scheduler,
        // el cambio de contexto y el reloj (uptime_ms) funcionan de verdad. Si se
        // ven ticks periódicos, la multitarea cooperativa está viva.
        println!(
            "[heartbeat] tick={} uptime={}ms led={}",
            tick,
            arch::xtensa::timer::uptime_ms(),
            encendido as u8
        );
        tick = tick.wrapping_add(1);
        sleep_ms(HEARTBEAT_MS);
    }
}

/// Espera aproximada de `ms` milisegundos cediendo la CPU a otras tareas.
///
/// Se apoya en el reloj monotónico del kernel (`arch::xtensa::timer::uptime_ms`)
/// y en `scheduler::yield_now` para no monopolizar el núcleo: es una espera
/// cooperativa, adecuada para una tarea de baja prioridad como el latido.
fn sleep_ms(ms: u64) {
    let inicio = arch::xtensa::timer::uptime_ms();
    while arch::xtensa::timer::uptime_ms().saturating_sub(inicio) < ms {
        scheduler::yield_now();
    }
}
