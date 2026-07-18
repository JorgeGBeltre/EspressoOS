#![no_std]
#![no_main]

use libc::{println, spawn, wait, yield_now};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // Nota: init NO es PID 1 todavía — es hijo del supervisor de kernel (que tomó el
    // pid bajo). El reparentado-a-PID-1 de verdad es SP4. El log no miente al respecto.
    println!("[init] up");

    // El script de arranque, corrido por el intérprete real (/bin/sh, que ahora tiene
    // modo-script). init ya no parsea /etc/rc: hay un solo intérprete, y /etc/rc gana
    // argv de verdad (su parser casero descartaba todos los argumentos). Si este spawn
    // falla, se reporta y se sigue a la consola igualmente: mejor un sh sin rc que
    // ninguna consola.
    run_and_wait(
        &[b"sh\0".as_ptr(), b"/etc/rc\0".as_ptr(), core::ptr::null()],
        "sh /etc/rc",
    );

    // La consola interactiva. Se relanza cada vez que SALE. Si en cambio no ARRANCA
    // tres veces seguidas, /bin/sh está roto de verdad: init sale con 1, el wait del
    // supervisor retorna, y el supervisor cae al shell del kernel. Esa cadena es el
    // fallback de SP1; no convertir esto en un reintento infinito (v1 lo era, y dejaba
    // la placa sin consola de serie con un /bin/sh corrupto).
    let mut failures = 0u32;
    loop {
        if run_and_wait(&[b"sh\0".as_ptr(), core::ptr::null()], "sh") {
            failures = 0;
        } else {
            failures += 1;
            if failures >= 3 {
                println!("[init] /bin/sh will not start; exiting so the supervisor falls back");
                return 1;
            }
            for _ in 0..100_000 {
                yield_now();
            }
        }
    }
}

/// Lanza /bin/sh con `argv` y espera a que termine. Devuelve false si el spawn en sí
/// falló (no arrancó), true si arrancó (haya salido como haya salido). `label` es sólo
/// para el log.
fn run_and_wait(argv: &[*const u8], label: &str) -> bool {
    let pid = spawn("/bin/sh", argv.as_ptr());
    if pid < 0 {
        println!("[init] ERROR spawning {}: {}", label, pid);
        return false;
    }
    let mut status = 0;
    let _ = wait(&mut status);
    true
}
