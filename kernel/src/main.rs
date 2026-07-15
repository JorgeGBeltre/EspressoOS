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
mod session;
mod shell;
mod syscall;
mod vfs;

mod wifi_credentials;

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

    let user_psram_size = 1024 * 1024;
    let heap_psram_base = unsafe { psram_base.add(user_psram_size) };
    let heap_psram_len = psram_len - user_psram_size;

    mm::heap::add_psram(heap_psram_base, heap_psram_len);
    mm::heap::init();

    mm::psram_exec::set_data_base(psram_base as usize as u32);
    println!(
        "[kernel] PSRAM added to heap: {} bytes @ {:p} (1MB reserved for Userland @ {:p})",
        heap_psram_len, heap_psram_base, psram_base
    );

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
                println!(
                    "[psram-exec] OK: code EXECUTED from PSRAM returned {} (expected 42)",
                    v
                );
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

    let flash_cap = drivers::flash::capacity() as u32;
    println!("[kernel] flash: {} MB usable", flash_cap / (1024 * 1024));
    if flash_cap < layout::FLASH_SIZE {
        println!(
            "[kernel] warning: image header declares {} MB but layout needs {} MB; \
             espfs (0x{:X}) and ota_0 (0x{:X}) are unreachable. Reflash with espflash.toml (size = \"16MB\").",
            flash_cap / (1024 * 1024),
            layout::FLASH_SIZE / (1024 * 1024),
            layout::FS_OFFSET,
            layout::OTA0_OFFSET
        );
    }

    match fs::EspFs::mount() {
        Ok(espfs) => match vfs::mount("/", espfs) {
            Ok(()) => println!("[kernel] / mounted on flash (espfs)"),
            Err(e) => {
                println!(
                    "[kernel] warning: mount / (espfs) failed: {:?}; using ramfs",
                    e
                );
                let _ = vfs::mount("/", fs::ramfs::RamFs::new());
            }
        },
        Err(e) => {
            println!(
                "[kernel] warning: EspFs::mount failed: {:?}; using ramfs on /",
                e
            );
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

    install_userland();

    init_etc_files();

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

    println!("[kernel] starting interactive console (kernel shell) on UART0...");

    // The serial console is session 0, built before the scheduler runs so the
    // shell task can be seeded with it. This is what makes the serial shell and
    // an SSH shell structurally identical: both own a pid, both own an fd table
    // whose 0/1/2 point at their own channel, and both hand that channel to their
    // children through clone_fd_table. The only difference left is which arm of
    // SessionChannel::write runs.
    let uart_chan = session::create(session::ChannelKind::Uart);
    match scheduler::spawn_blocked(
        "shell",
        shell_task,
        uart_chan.id as usize,
        layout::DEFAULT_STACK_SIZE,
        PRIO_DEFAULT,
        false,
    ) {
        Ok(tid) => {
            let pid = scheduler::process::register_process(
                "shell",
                tid,
                false,
                core::ptr::null_mut(),
                0,
            );
            if let Err(e) = vfs::seed_fd_table(pid, session::SessionConsole::new(uart_chan)) {
                println!("[kernel] warning: could not seed shell fd table: {:?}", e);
            }
            scheduler::unblock_task(tid);
            println!("[kernel] task 'shell' created (tid={}, pid={})", tid, pid);
        }
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
    match scheduler::spawn(
        "net",
        drivers::wifi::net_task,
        0,
        NET_STACK_SIZE,
        PRIO_DEFAULT,
        false,
    ) {
        Ok(tid) => println!("[kernel] task 'net' created (tid={})", tid),
        Err(e) => println!("[kernel] warning: could not create net: {:?}", e),
    }

    let _ = arch::xtensa::interrupts::disable();
    arch::xtensa::timer::init();

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
    // `exit` at the prompt makes run_session return: it breaks out before read_line
    // is ever consulted, so "the UART never reports EOF" does not cover that path.
    // Without this loop one `exit` ends the task, reap_orphans tears down pid 1,
    // and the console is gone until a hardware reset -- on a board whose only local
    // way in is this port. Looping gives back what it did before: log out, print
    // the banner, start over. No spin risk: the UART channel never reports EOF, so
    // the only way round the loop is a deliberate `exit`.
    loop {
        shell::run_session(None);
    }
}

fn heartbeat_task(_arg: usize) {
    let _ = drivers::gpio::configure(LED_GPIO, drivers::gpio::PinMode::Output);

    let mut encendido = false;
    loop {
        encendido = !encendido;
        let _ = drivers::gpio::write(LED_GPIO, encendido);

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

    let _ = vfs::mkdir("/bin");

    let mut n = 0u32;
    for (name, bytes) in userland_bin::USERLAND_BINARIES {
        let path = alloc::format!("/bin/{}", name);

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

        let _ = vfs::unlink(&path);

        let flags = vfs::OpenFlags(
            vfs::OpenFlags::WRONLY.0 | vfs::OpenFlags::CREATE.0 | vfs::OpenFlags::TRUNC.0,
        );
        match vfs::open(&path, flags) {
            Ok(fd) => {
                let _ = vfs::write(fd, bytes);
                let _ = vfs::close(fd);
                n += 1;
                println!(
                    "[kernel] Deployed /bin/{} ({} bytes) to EspFs",
                    name,
                    bytes.len()
                );
            }
            Err(e) => println!("[kernel] warning: install {} failed: {:?}", path, e),
        }
    }
    if n > 0 {
        println!(
            "[kernel] userland: {} binaries installed/updated in EspFs",
            n
        );
    } else {
        println!("[kernel] userland: all binaries are up to date in EspFs");
    }
}

fn init_etc_files() {
    let _ = vfs::mkdir("/etc");

    if let Err(_) = vfs::mount::resolve("/etc/rc") {
        if let Ok(fd) = vfs::open(
            "/etc/rc",
            vfs::OpenFlags(vfs::OpenFlags::CREATE.0 | vfs::OpenFlags::WRONLY.0),
        ) {
            let rc_content =
                b"# EspressoOS Startup Script\n/bin/echo [rc] System started!\n/bin/ls\n";
            let _ = vfs::write(fd, rc_content);
            let _ = vfs::close(fd);
        }
    }

    // Never seed /etc/passwd. drivers/ssh/auth.rs consults it BEFORE the compiled
    // DEV_USER/DEV_PASSWORD and returns Success on a match, so seeding it with
    // defaults -- it used to be "root:root\nguest:guest\n" -- put two plaintext
    // accounts on every board that changing DEV_PASSWORD did not close. And since
    // it was only written when absent, a stale one survives a re-flash now that
    // EspFs persists.
    //
    // A file that exists from here on is deliberate, so leave it alone and say so.
    // Passwords in it are compared in plaintext.
    if vfs::mount::resolve("/etc/passwd").is_ok() {
        println!(
            "[kernel] WARNING: /etc/passwd exists and overrides the compiled SSH credential; \
             'rm /etc/passwd' to fall back to it"
        );
    }
}
