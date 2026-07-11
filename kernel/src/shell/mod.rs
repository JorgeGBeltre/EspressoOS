//! Shell interactiva: bucle REPL, parser y comandos internos.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Lee líneas de la consola carácter a carácter (`drivers::uart`), las parsea
//! (`parser`) y despacha a los comandos internos (`commands`). Soporta
//! redirecciones simples (`>`, `>>`) delegando en el sink de `commands`, y
//! detecta tuberías (`|`) aunque su ejecución encadenada queda para una fase
//! posterior (requiere plumbing de E/S entre procesos).
//!
//! Corre como una tarea del planificador; cuando no hay entrada disponible cede
//! la CPU con `scheduler::yield_now()` (shell cooperativa).
#![allow(dead_code)]

pub mod commands;
pub mod parser;

use crate::drivers::uart;
use crate::prelude::*;
use crate::scheduler;
use alloc::format;

/// Texto del prompt.
const PROMPT: &str = "esp32s3-os> ";

/// Longitud máxima de una línea de entrada (evita crecer sin límite).
const MAX_LINE: usize = 256;

/// Bucle principal de la shell (REPL). [CANÓNICO]
///
/// No retorna en operación normal (bucle infinito). Firma sin `!` para
/// permitir, en el futuro, una salida limpia (p. ej. comando `exit`).
pub fn run() {
    banner();
    let mut line = String::new();
    loop {
        print_prompt();
        line.clear();
        read_line(&mut line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        execute(trimmed);
    }
}

/// Imprime el banner de bienvenida (una vez, al arrancar la shell).
fn banner() {
    console_write(b"\r\n");
    console_write(b"esp32s3-os shell. Escribe 'help' para ver los comandos.\r\n");
}

/// Lee una línea de la consola hasta un salto de línea.
///
/// Hace eco de los caracteres imprimibles, gestiona borrado (Backspace/DEL) y
/// cancelación (Ctrl-C). Si no hay bytes disponibles, cede la CPU.
fn read_line(buf: &mut String) {
    loop {
        match uart::getc() {
            Some(byte) => match byte {
                b'\r' | b'\n' => {
                    console_write(b"\r\n");
                    return;
                }
                0x08 | 0x7f => {
                    // Backspace / DEL: borra el último carácter, si lo hay.
                    if buf.pop().is_some() {
                        // Retrocede, pinta espacio y retrocede de nuevo.
                        console_write(b"\x08 \x08");
                    }
                }
                0x03 => {
                    // Ctrl-C: descarta la línea en curso.
                    buf.clear();
                    console_write(b"^C\r\n");
                    return;
                }
                b if (0x20..0x7f).contains(&b) => {
                    // Carácter ASCII imprimible: lo añadimos si hay sitio.
                    if buf.len() < MAX_LINE {
                        buf.push(b as char);
                        console_write(&[b]);
                    }
                    // Si se supera MAX_LINE, se ignora (sin eco) para no
                    // desbordar el buffer de la línea.
                }
                _ => {
                    // Otros controles no imprimibles: se ignoran.
                }
            },
            None => {
                // Sin datos: cedemos la CPU a otras tareas (cooperativo).
                scheduler::yield_now();
            }
        }
    }
}

/// Parsea y ejecuta una línea (ya recortada y no vacía).
fn execute(line: &str) {
    match parser::parse_pipeline(line) {
        Ok(pipeline) => {
            if pipeline.is_empty() {
                return;
            }
            if pipeline.len() > 1 {
                // Las tuberías se detectan, pero todavía no conectan la E/S
                // entre etapas (requiere plumbing de procesos, fase posterior).
                // Se ejecuta sólo la primera etapa.
                eprintln_console(
                    "shell: tuberías aún no soportadas; ejecutando la primera etapa",
                );
            }
            if let Some(cmd) = pipeline.into_iter().next() {
                run_command(&cmd);
            }
        }
        Err(e) => {
            eprintln_console(&format!("shell: error de sintaxis ({:?})", e));
        }
    }
}

/// Ejecuta un comando ya parseado, aplicando su redirección de salida.
fn run_command(cmd: &parser::Command) {
    // Configura el destino de salida (abre el archivo si hay redirección).
    if let Err(e) = commands::begin_redirect(&cmd.redirect) {
        eprintln_console(&format!(
            "shell: no se pudo abrir el destino de redirección ({:?})",
            e
        ));
        return;
    }
    // Construye las vistas `&str` de los argumentos que espera `dispatch`.
    let args: Vec<&str> = cmd.args.iter().map(|s| s.as_str()).collect();
    let _code = commands::dispatch(cmd.name.as_str(), &args);
    // Cierra la redirección y vuelve a consola pase lo que pase.
    commands::end_redirect();
}

// ===========================================================================
// Salida a consola (prompt, eco y diagnósticos de la propia shell).
// ===========================================================================

/// Escribe bytes crudos en la consola.
fn console_write(bytes: &[u8]) {
    let _ = uart::write(bytes);
}

/// Imprime el prompt.
fn print_prompt() {
    console_write(PROMPT.as_bytes());
}

/// Imprime una línea de diagnóstico de la shell en la consola.
fn eprintln_console(s: &str) {
    console_write(s.as_bytes());
    console_write(b"\r\n");
}
