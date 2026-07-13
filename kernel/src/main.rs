#![no_std]
#![no_main]

#![feature(asm_experimental_arch)]
#![allow(dead_code, unused_imports, unused_variables)]

extern crate alloc;

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

mod wifi_credentials;

use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::main;
use esp_println::println;

use crate::prelude::*;

const LED_GPIO: u8 = 2;

const PRIO_DEFAULT: u8 = 1;

const HEARTBEAT_MS: u64 = 500;

const NET_STACK_SIZE: usize = 16 * 1024;

#[main]
fn main() -> ! {

    let peripherals = esp_hal::init(
        esp_hal::Config::default()
            .with_cpu_clock(CpuClock::max())
            .with_psram(esp_hal::psram::PsramConfig {

                size: esp_hal::psram::PsramSize::Size(8 * 1024 * 1024),
                ..Default::default()
            }),
    );

    mm::heap::init();

    let (psram_base, psram_len) = esp_hal::psram::psram_raw_parts(&peripherals.PSRAM);
    mm::heap::add_psram(psram_base, psram_len);
    println!(
        "[kernel] PSRAM añadida al heap: {} bytes @ {:p}",
        psram_len, psram_base
    );

    if let Err(e) = drivers::uart::init() {
        println!("[kernel] aviso: drivers::uart::init fallo: {:?}", e);
    }

    banner();

    arch::xtensa::interrupts::init();

    mm::mpu::init();

    if let Err(e) = vfs::init() {
        println!("[kernel] aviso: vfs::init fallo: {:?}", e);
    }

    match vfs::devfs::init() {
        Ok(devfs) => {
            if let Err(e) = vfs::mount("/dev", devfs) {
                println!("[kernel] aviso: mount /dev fallo: {:?}", e);
            }
        }
        Err(e) => println!("[kernel] aviso: devfs::init fallo: {:?}", e),
    }

    // '/' persistente en flash (EspFs, Fase 4). Fallback a ramfs para no dejar
    // de arrancar si el flash/superbloque fallan.
    match fs::EspFs::mount() {
        Ok(espfs) => match vfs::mount("/", espfs) {
            Ok(()) => println!("[kernel] / montado en flash (espfs)"),
            Err(e) => {
                println!("[kernel] aviso: mount / (espfs) fallo: {:?}; usando ramfs", e);
                let _ = vfs::mount("/", fs::ramfs::RamFs::new());
            }
        },
        Err(e) => {
            println!("[kernel] aviso: EspFs::mount fallo: {:?}; usando ramfs en /", e);
            if let Err(e2) = vfs::mount("/", fs::ramfs::RamFs::new()) {
                println!("[kernel] aviso: mount / (ramfs) fallo: {:?}", e2);
            }
        }
    }
    if let Err(e) = vfs::mount("/tmp", fs::ramfs::RamFs::new()) {
        println!("[kernel] aviso: mount /tmp fallo: {:?}", e);
    }

    // Buses I2C/SPI (Fase 3): periféricos entregados desde aquí.
    if let Err(e) = drivers::i2c::init(peripherals.I2C0, peripherals.GPIO8, peripherals.GPIO9) {
        println!("[kernel] aviso: i2c::init fallo: {:?}", e);
    }
    if let Err(e) = drivers::spi::init(
        peripherals.SPI2,
        peripherals.GPIO12,
        peripherals.GPIO11,
        peripherals.GPIO13,
    ) {
        println!("[kernel] aviso: spi::init fallo: {:?}", e);
    }

    scheduler::init();

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

    let _ = arch::xtensa::interrupts::disable();
    arch::xtensa::timer::init();

    // SMP (Fase 9, opt-in): arranca el segundo núcleo antes del planificador.
    #[cfg(feature = "smp")]
    scheduler::core_sync::start_secondary_core(peripherals.CPU_CTRL);

    println!("[kernel] arrancando el planificador...");

    scheduler::run();
}

fn banner() {
    println!();
    println!("========================================");
    println!("   esp32s3-os   ·   kernel");
    println!("   Consola viva. Arrancando subsistemas.");
    println!("   Heap del kernel: {} bytes", mm::heap::size());
    println!("========================================");
}

fn shell_task(_arg: usize) {
    shell::run();
}

fn heartbeat_task(_arg: usize) {

    let _ = drivers::gpio::configure(LED_GPIO, drivers::gpio::PinMode::Output);

    let mut encendido = false;
    let mut tick: u64 = 0;
    loop {
        encendido = !encendido;
        let _ = drivers::gpio::write(LED_GPIO, encendido);

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

fn sleep_ms(ms: u64) {
    let inicio = arch::xtensa::timer::uptime_ms();
    while arch::xtensa::timer::uptime_ms().saturating_sub(inicio) < ms {
        scheduler::yield_now();
    }
}
