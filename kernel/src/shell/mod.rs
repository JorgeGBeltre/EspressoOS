#![allow(dead_code)]

pub mod commands;
pub mod parser;
pub mod remote;

use crate::drivers::uart;
use crate::prelude::*;
use crate::scheduler;
use alloc::format;

const PROMPT: &str = "EspressoOS> ";

const MAX_LINE: usize = 256;

pub fn run() {
    remote::run_with_io(&mut remote::ConsoleIo);
}

pub(crate) fn banner_bytes() -> &'static [u8] {
    b"\r\nEspressoOS shell. Type 'help' to see the commands.\r\n"
}


pub(crate) fn prompt_bytes() -> &'static [u8] {
    PROMPT.as_bytes()
}

pub(crate) const MAX_LINE_LEN: usize = MAX_LINE;

pub(crate) fn execute_line(line: &str) {
    execute(line);
}

fn banner() {
    console_write(b"\r\n");
    console_write(b"EspressoOS shell. Type 'help' to see the commands.\r\n");
}

fn read_line(buf: &mut String) {
    loop {
        match uart::getc() {
            Some(byte) => match byte {
                b'\r' | b'\n' => {
                    console_write(b"\r\n");
                    return;
                }
                0x08 | 0x7f => {

                    if buf.pop().is_some() {

                        console_write(b"\x08 \x08");
                    }
                }
                0x03 => {

                    buf.clear();
                    console_write(b"^C\r\n");
                    return;
                }
                b if (0x20..0x7f).contains(&b) => {

                    if buf.len() < MAX_LINE {
                        buf.push(b as char);
                        console_write(&[b]);
                    }

                }
                _ => {

                }
            },
            None => {

                scheduler::yield_now();
            }
        }
    }
}

fn execute(line: &str) {
    match parser::parse_pipeline(line) {
        Ok(pipeline) => {
            if pipeline.is_empty() {
                return;
            }
            if pipeline.len() > 1 {

                eprintln_console(
                    "shell: pipelines not yet supported; running the first stage",
                );
            }
            if let Some(cmd) = pipeline.into_iter().next() {
                run_command(&cmd);
            }
        }
        Err(e) => {
            eprintln_console(&format!("shell: syntax error ({:?})", e));
        }
    }
}

fn run_command(cmd: &parser::Command) {

    if let Err(e) = commands::begin_redirect(&cmd.redirect) {
        eprintln_console(&format!(
            "shell: could not open redirection target ({:?})",
            e
        ));
        return;
    }

    let args: Vec<&str> = cmd.args.iter().map(|s| s.as_str()).collect();
    let _code = commands::dispatch(cmd.name.as_str(), &args);

    commands::end_redirect();
}

fn console_write(bytes: &[u8]) {
    let _ = uart::write(bytes);
}

fn print_prompt() {
    console_write(PROMPT.as_bytes());
}

fn eprintln_console(s: &str) {
    console_write(s.as_bytes());
    console_write(b"\r\n");
}
