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

// Binarios de userland empotrados por build.rs (userland/dist/*.elf). Si no se
// corrió tools/build-userland.ps1, la tabla queda vacía y se usa la shell interna.
mod userland_bin {
    include!(concat!(env!("OUT_DIR"), "/userland_bin.rs"));
}

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

    let (psram_base, psram_len) = esp_hal::psram::psram_raw_parts(&peripherals.PSRAM);

    // Reservar el primer 1 MB de PSRAM (0x3c000000) para binarios estáticos de usuario (userland)
    let user_psram_size = 1024 * 1024;
    let heap_psram_base = unsafe { psram_base.add(user_psram_size) };
    let heap_psram_len = psram_len - user_psram_size;

    // ORDEN IMPORTANTE: PSRAM al heap PRIMERO, RAM interna DESPUÉS. esp_alloc prueba
    // las regiones en orden de alta y `GlobalAlloc::alloc` usa caps EMPTY (cualquier
    // región), así que las asignaciones GENERALES (ramfs /bin ~140KB, tablas, etc.)
    // van a PSRAM y la RAM interna (KERNEL_HEAP, 128KB) queda LIBRE para esp-wifi,
    // que exige memoria `Internal` para la pila de su task de WiFi (si no la hay,
    // su `malloc` devuelve NULL y crashea en task_create con StoreProhibited).
    mm::heap::add_psram(heap_psram_base, heap_psram_len);
    mm::heap::init();
    // Base de datos de la región de userland (alias de escritura del .text).
    mm::psram_exec::set_data_base(psram_base as usize as u32);
    println!(
        "[kernel] PSRAM added to heap: {} bytes @ {:p} (1MB reserved for Userland @ {:p})",
        heap_psram_len, heap_psram_base, psram_base
    );

    // Ruta B (userland ejecutable): mapea el 1 MB de PSRAM reservado (@psram_base,
    // páginas físicas 0..N) también al bus de INSTRUCCIONES, y autotesta que se
    // puede EJECUTAR desde PSRAM. Paso 1 antes de cablear el loader de dos regiones.
    let user_pages = (user_psram_size / mm::psram_exec::MMU_PAGE_SIZE as usize) as u32;
    match mm::psram_exec::map_instruction(0, user_pages) {
        Ok(()) => {
            println!(
                "[psram-exec] reserved PSRAM mapped to the instruction bus @ {:#x} ({} pages)",
                mm::psram_exec::USER_IBUS_BASE,
                user_pages
            );
            let v = mm::psram_exec::selftest(psram_base as usize as u32);
            if v == 42 {
                println!("[psram-exec] OK: code EXECUTED from PSRAM returned {} (expected 42)", v);
            } else {
                println!("[psram-exec] FAILED: returned {} (expected 42)", v);
            }
        }
        Err(code) => println!("[psram-exec] ERROR: Cache_Ibus_MMU_Set returned {}", code),
    }

    drivers::power::init(peripherals.LPWR);
    drivers::device::init();

    if let Err(e) = drivers::uart::init() {
        println!("[kernel] warning: drivers::uart::init failed: {:?}", e);
    }

    banner();

    arch::xtensa::interrupts::init();

    mm::mpu::init();

    if let Err(e) = vfs::init() {
        println!("[kernel] warning: vfs::init failed: {:?}", e);
    }

    match vfs::devfs::init() {
        Ok(devfs) => {
            if let Err(e) = vfs::mount("/dev", devfs) {
                println!("[kernel] warning: mount /dev failed: {:?}", e);
            }
        }
        Err(e) => println!("[kernel] warning: devfs::init failed: {:?}", e),
    }

    // '/' persistente en flash (EspFs, Fase 4). Fallback a ramfs para no dejar
    // de arrancar si el flash/superbloque fallan.
    match fs::EspFs::mount() {
        Ok(espfs) => match vfs::mount("/", espfs) {
            Ok(()) => println!("[kernel] / mounted on flash (espfs)"),
            Err(e) => {
                println!("[kernel] warning: mount / (espfs) failed: {:?}; using ramfs", e);
                let _ = vfs::mount("/", fs::ramfs::RamFs::new());
            }
        },
        Err(e) => {
            println!("[kernel] warning: EspFs::mount failed: {:?}; using ramfs on /", e);
            if let Err(e2) = vfs::mount("/", fs::ramfs::RamFs::new()) {
                println!("[kernel] warning: mount / (ramfs) failed: {:?}", e2);
            }
        }
    }
    if let Err(e) = vfs::mount("/tmp", fs::ramfs::RamFs::new()) {
        println!("[kernel] warning: mount /tmp failed: {:?}", e);
    }
    if let Err(e) = vfs::mount("/proc", alloc::sync::Arc::new(fs::ProcFs::new())) {
        println!("[kernel] warning: mount /proc failed: {:?}", e);
    }
    if let Err(e) = vfs::mount("/sys", alloc::sync::Arc::new(fs::SysFs::new())) {
        println!("[kernel] warning: mount /sys failed: {:?}", e);
    }

    // /bin: ramfs poblado con los binarios de userland empotrados en el firmware.
    install_userland();

    init_etc_files();

    // Buses I2C/SPI (Fase 3): periféricos entregados desde aquí.
    if let Err(e) = drivers::i2c::init(peripherals.I2C0, peripherals.GPIO8, peripherals.GPIO9) {
        println!("[kernel] warning: i2c::init failed: {:?}", e);
    }
    if let Err(e) = drivers::spi::init(
        peripherals.SPI2,
        peripherals.GPIO12,
        peripherals.GPIO11,
        peripherals.GPIO13,
    ) {
        println!("[kernel] warning: spi::init failed: {:?}", e);
    }

    scheduler::init();

    // Consola interactiva: el shell del KERNEL sobre UART0 — mismos comandos que
    // por SSH (wifi, ls, help, cd, cat, i2c, ota...), con parseo de argumentos.
    // El userland sigue desplegado en /bin (install_userland) y su maquinaria
    // (loader ELF, ejecución desde PSRAM, spawn/wait) permanece en el código; se
    // inspecciona con `ls /bin` / `cat`.
    println!("[kernel] starting interactive console (kernel shell) on UART0...");
    match scheduler::spawn("shell", shell_task, 0, layout::DEFAULT_STACK_SIZE, PRIO_DEFAULT, false) {
        Ok(tid) => println!("[kernel] task 'shell' created (tid={})", tid),
        Err(e) => println!("[kernel] ERROR: could not create shell: {:?}", e),
    }

    match scheduler::spawn(
        "heartbeat",
        heartbeat_task,
        0,
        layout::DEFAULT_STACK_SIZE,
        PRIO_DEFAULT,
        false,
    ) {
        Ok(tid) => println!("[kernel] task 'heartbeat' created (tid={})", tid),
        Err(e) => println!("[kernel] warning: could not create heartbeat: {:?}", e),
    }

    drivers::wifi::provide_peripherals(
        peripherals.TIMG0,
        peripherals.RNG,
        peripherals.RADIO_CLK,
        peripherals.WIFI,
        peripherals.BT,
    );
    match scheduler::spawn("net", drivers::wifi::net_task, 0, NET_STACK_SIZE, PRIO_DEFAULT, false) {
        Ok(tid) => println!("[kernel] task 'net' created (tid={})", tid),
        Err(e) => println!("[kernel] warning: could not create net: {:?}", e),
    }

    let _ = arch::xtensa::interrupts::disable();
    arch::xtensa::timer::init();

    // SMP (Fase 9, opt-in): encola una tarea para el núcleo 1 y arráncalo.
    #[cfg(feature = "smp")]
    {
        let _ = scheduler::spawn_core1(
            "core1-worker",
            scheduler::core_sync::worker_entry,
            0,
            layout::DEFAULT_STACK_SIZE,
            PRIO_DEFAULT,
        );
        scheduler::core_sync::start_secondary_core(peripherals.CPU_CTRL);
    }

    println!("[kernel] starting the scheduler...");

    scheduler::run();
}

fn banner() {
    println!();
    println!("========================================");
    println!("   EspressoOS   ·   kernel");
    println!("   Live console. Starting subsystems.");
    println!("   Kernel heap: {} bytes", mm::heap::size());
    println!("========================================");
}

fn shell_task(_arg: usize) {
    // Reinicia el shell si `run` retorna (p.ej. al teclear `exit` en la consola),
    // para no dejar la consola muerta.
    loop {
        shell::run();
        scheduler::yield_now();
    }
}

fn heartbeat_task(_arg: usize) {

    let _ = drivers::gpio::configure(LED_GPIO, drivers::gpio::PinMode::Output);

    let mut encendido = false;
    loop {
        encendido = !encendido;
        let _ = drivers::gpio::write(LED_GPIO, encendido);

        // Sin traza por serial: el heartbeat inundaba la consola y corrompía la
        // escritura interactiva en el shell. El parpadeo del LED sigue siendo la
        // prueba visual de que la multitarea preemptiva está viva.
        sleep_ms(HEARTBEAT_MS);
    }
}

fn sleep_ms(ms: u64) {
    let inicio = arch::xtensa::timer::uptime_ms();
    while arch::xtensa::timer::uptime_ms().saturating_sub(inicio) < ms {
        scheduler::yield_now();
    }
}

fn install_userland() {
    if userland_bin::USERLAND_BINARIES.is_empty() {
        println!("[kernel] userland not embedded");
        return;
    }
    
    // Asegurar existencia del directorio /bin en EspFs
    let _ = vfs::mkdir("/bin");

    let mut n = 0u32;
    for (name, bytes) in userland_bin::USERLAND_BINARIES {
        let path = alloc::format!("/bin/{}", name);
        
        // Comprobar si el archivo ya existe en EspFs y coincide en tamaño
        let mut match_exists = false;
        let read_flags = vfs::OpenFlags(vfs::OpenFlags::RDONLY.0);
        if let Ok(fd) = vfs::open(&path, read_flags) {
            if let Ok(inode) = vfs::get_inode(fd) {
                if inode.size() == bytes.len() as u64 {
                    match_exists = true;
                }
            }
            let _ = vfs::close(fd);
        }
        
        if match_exists {
            continue;
        }
        
        // Borrar el archivo viejo si ya existía con otro tamaño
        let _ = vfs::unlink(&path);
        
        let flags = vfs::OpenFlags(
            vfs::OpenFlags::WRONLY.0 | vfs::OpenFlags::CREATE.0 | vfs::OpenFlags::TRUNC.0,
        );
        match vfs::open(&path, flags) {
            Ok(fd) => {
                let _ = vfs::write(fd, bytes);
                let _ = vfs::close(fd);
                n += 1;
                println!("[kernel] Deployed /bin/{} ({} bytes) to EspFs", name, bytes.len());
            }
            Err(e) => println!("[kernel] warning: install {} failed: {:?}", path, e),
        }
    }
    if n > 0 {
        println!("[kernel] userland: {} binaries installed/updated in EspFs", n);
    } else {
        println!("[kernel] userland: all binaries are up to date in EspFs");
    }
}

fn init_etc_files() {
    let _ = vfs::mkdir("/etc");
    
    // Crear /etc/rc si no existe
    if let Err(_) = vfs::mount::resolve("/etc/rc") {
        if let Ok(fd) = vfs::open("/etc/rc", vfs::OpenFlags(vfs::OpenFlags::CREATE.0 | vfs::OpenFlags::WRONLY.0)) {
            let rc_content = b"# EspressoOS Startup Script\n/bin/echo [rc] System started!\n/bin/ls\n";
            let _ = vfs::write(fd, rc_content);
            let _ = vfs::close(fd);
        }
    }
    
    // Crear /etc/passwd si no existe
    if let Err(_) = vfs::mount::resolve("/etc/passwd") {
        if let Ok(fd) = vfs::open("/etc/passwd", vfs::OpenFlags(vfs::OpenFlags::CREATE.0 | vfs::OpenFlags::WRONLY.0)) {
            let passwd_content = b"root:root\nguest:guest\n";
            let _ = vfs::write(fd, passwd_content);
            let _ = vfs::close(fd);
        }
    }
}
