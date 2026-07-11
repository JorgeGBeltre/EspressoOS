//! Interfaz de llamadas al sistema (frontera kernel/usuario).
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Los programas invocan servicios del kernel (fs, proc, io, time, mem) sin
//! tocar hardware. Esta capa NO ejecuta lógica de negocio: normaliza el ABI y
//! delega en los subsistemas (`vfs`, `scheduler`, `mm`, `arch::xtensa::timer`)
//! a través de [`handler::dispatch`].
//!
//! # ABI de syscalls en Xtensa LX7
//!
//! La entrada al kernel se produce con la instrucción `syscall` de Xtensa, que
//! provoca una excepción atendida por el vector `Syscall` (instalado por
//! `arch::xtensa::interrupts`, Fase 6). El manejador de bajo nivel (en
//! ensamblador) es quien traduce registros a la llamada Rust; esta es la
//! convención acordada:
//!
//! - **Número de syscall**: registro `a2` (uno de los discriminantes de
//!   [`table::Syscall`], `#[repr(usize)]`).
//! - **Argumentos** (hasta 6): registros `a3, a4, a5, a6, a7, a8`, en ese orden.
//! - **Valor de retorno**: se deja en `a2` al volver de la excepción.
//!   Es un `isize`: `>= 0` éxito (bytes, fd, tid, posición…); `< 0` error, igual
//!   a `KError::as_errno()` (errno negativo). El código de usuario reconstruye el
//!   error negando el valor.
//!
//! El vector recoge `a3..a8` en un `[usize; 6]` en la pila de la excepción y
//! llama a `handler::dispatch(a2, &args)`. Punteros y longitudes viajan como
//! `usize` crudos; sin MMU, el kernel los desreferencia directamente (véase la
//! reconstrucción de buffers en [`handler`]).
//!
//! ## Alternativa `break`
//!
//! Para depuración puede usarse la instrucción `break 1, 15` (trap de software)
//! en lugar de `syscall`; el vector de excepción de depuración reencaminaría al
//! mismo `dispatch`. La ruta canónica de producción es `syscall`.
//!
//! ## Correspondencia número -> subsistema (§8 del contrato)
//!
//! | Syscall(s)                                             | Subsistema                     |
//! |--------------------------------------------------------|--------------------------------|
//! | `Read`/`Write`/`Open`/`Close`/`Seek`/`Mkdir`/`Unlink`/`Readdir`/`Ioctl` | `vfs::*`      |
//! | `Exit`/`Spawn`/`Wait`/`Yield`                          | `scheduler::*`                 |
//! | `UptimeMs`                                             | `arch::xtensa::timer::uptime_ms` |
//! | `Sbrk`                                                 | `mm` (consulta de heap)        |
//!
//! El resto del kernel solo necesita conocer [`handler::dispatch`]; el detalle de
//! registros queda confinado al vector de excepción.
#![allow(dead_code)]

pub mod handler;
pub mod table;

// Re-exports de conveniencia para el vector de excepción y las pruebas.
pub use handler::dispatch;
pub use table::Syscall;
