#![no_std]
#![no_main]

use libc::{chdir, getcwd, println, syscall, PATH_MAX};

// Puntero inválido reutilizado de badptr.rs (Check 6 del plan): 0xDEAD_BEEF no está
// mapeado, así que validate_user lo rechaza con EFAULT. Mismo criterio que badptr.
const BAD_PTR: usize = 0xDEAD_BEEF;

const SYS_CHDIR: usize = 28;
const SYS_GETCWD: usize = 29;

fn check(name: &str, ok: bool, fails: &mut i32) {
    if ok {
        println!("[cwdtest] OK   {}", name);
    } else {
        println!("[cwdtest] FAIL {}", name);
        *fails += 1;
    }
}

/// Verifica la frontera de las dos syscalls nuevas (chdir/getcwd): EFAULT ante punteros
/// ajenos, ENOENT/no-es-directorio sin mover el cwd, bordes exactos de buffer, y las
/// rutas relativas (`..`). Misma filosofía que badptr: el programa conoce todas las
/// respuestas correctas, así que ningún caso puede pasar por accidente.
///
/// Asume cwd == "/" al arrancar (lanzarlo tras `cd /`). Corre en su propio proceso, así
/// que sus `chdir` NO afectan al cwd del shell que lo lanzó.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut fails = 0;
    let mut buf = [0u8; PATH_MAX];

    // Frontera: punteros que el kernel debe rechazar (estilo badptr) -> -errno.
    let r = unsafe { syscall(SYS_CHDIR, BAD_PTR, 16, 0, 0, 0, 0) };
    check("chdir(bad ptr) -> -errno", r < 0, &mut fails);
    let r = unsafe { syscall(SYS_GETCWD, BAD_PTR, 64, 0, 0, 0, 0) };
    check("getcwd(bad ptr) -> -errno", r < 0, &mut fails);

    // Validación semántica: no-existe / no-es-directorio, sin mover el cwd.
    check("chdir(/noexiste) -> -errno", chdir("/noexiste") < 0, &mut fails);
    check("chdir(/etc/rc, un fichero) -> -errno", chdir("/etc/rc") < 0, &mut fails);
    let n = getcwd(&mut buf);
    check("cwd intacto tras los fallos (== /)", n == 1 && buf[0] == b'/', &mut fails);

    // Camino feliz.
    check("chdir(/tmp) -> 0", chdir("/tmp") == 0, &mut fails);
    let n = getcwd(&mut buf);
    check("getcwd == /tmp", n == 4 && &buf[..4] == b"/tmp", &mut fails);

    // Bordes exactos del buffer (cwd == "/tmp", 4 bytes).
    let mut exact = [0u8; 4];
    check("getcwd(size == len) -> len", getcwd(&mut exact) == 4, &mut fails);
    let mut small = [0u8; 3];
    check("getcwd(size == len-1) -> -errno", getcwd(&mut small) < 0, &mut fails);
    let mut zero = [0u8; 0];
    check("getcwd(size == 0) -> -errno", getcwd(&mut zero) < 0, &mut fails);

    // Relativas: normalize colapsa `..` (verificado en vfs/mount.rs).
    check("chdir(..) desde /tmp -> 0", chdir("..") == 0, &mut fails);
    let n = getcwd(&mut buf);
    check("tras .., cwd == /", n == 1 && buf[0] == b'/', &mut fails);

    if fails == 0 {
        println!("[cwdtest] all tests passed");
        0
    } else {
        println!("[cwdtest] {} failure(s)", fails);
        1
    }
}
