#![no_std]
#![no_main]

use libc::{arg, chdir, close, dup2, getcwd_str, open, print, println, read, readdir, spawn, wait, yield_now, pipe, PATH_MAX};

/// Most tokens one command can have, counting argv[0] and the trailing NULL.
const MAX_ARGV: usize = 16;

/// Longest command line accepted. `LINE + 1` bytes are kept: `tokenize` writes the
/// last token's NUL one past the end of the range it is given.
const LINE: usize = 64;

const O_RDONLY: u32 = 1;
const O_WRONLY: u32 = 0x0002;
const O_CREATE: u32 = 0x0100;
const O_APPEND: u32 = 0x0200;
const O_TRUNC: u32 = 0x0400;

/// Máximo de etapas en un pipeline (`a | b | c | ...`).
const MAX_STAGES: usize = 8;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // Modo-script: `sh <fichero>` corre el fichero y sale. Sin argumentos = interactivo.
    if argc > 1 {
        let path = unsafe { arg(argv, 1) };
        return run_script(path);
    }

    println!("--- EspressoOS Shell (Userland) ---");
    let mut buf = [0u8; LINE + 1];
    loop {
        print_prompt();
        let len = read_line(&mut buf[..LINE]);
        let (s, e) = trim(&buf, 0, len);
        if !run_line(&mut buf, s, e) {
            println!("Exiting the shell...");
            break;
        }
    }
    0
}

/// Tamaño máximo de script. Vive en la pila de `sh` y sigue vivo A TRAVÉS de `spawn`
/// (la syscall más profunda — la que desbordó los 8K originales). Si `/etc/rc` crece,
/// mover el buffer a `static`, NO agrandarlo aquí en pila.
const SCRIPT_MAX: usize = 1024;

/// Ejecuta cada línea del fichero en `path` y retorna. Semántica DELIBERADA (no
/// "arreglar" hacia abort-on-error): una línea que falla se reporta y el script
/// CONTINÚA — un `ls` roto en `/etc/rc` no debe impedir llegar a la consola. Salta
/// vacías y comentarios ('#'), recorta '\r' final (CRLF de hosts Windows), honra
/// `exit`, y NUNCA trunca en silencio: fichero sobredimensionado = error ruidoso y
/// exit 1; línea más larga que `LINE` = se reporta y se salta entera.
///
/// Enruta por `run_line` (NO `exec_line`) para conservar la paridad de `;` que el
/// modo interactivo ya tiene: `echo uno ; echo dos` en un script se comporta igual.
fn run_script(path: &str) -> i32 {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        println!("sh: cannot open {}", path);
        return 1;
    }
    let mut file = [0u8; SCRIPT_MAX];
    // Bucle de lectura: un solo `read` puede devolver corto sin ser EOF.
    let mut total = 0usize;
    loop {
        if total == file.len() {
            // El buffer se llenó. ¿Queda más fichero? Un byte de sonda lo decide: si lo
            // hay, el script no cabe y NO se ejecuta medio — error ruidoso y exit 1.
            let mut probe = [0u8; 1];
            if read(fd as i32, &mut probe) > 0 {
                close(fd as i32);
                println!(
                    "sh: {}: larger than {} bytes; refusing to run a truncated script",
                    path, SCRIPT_MAX
                );
                return 1;
            }
            break;
        }
        let n = read(fd as i32, &mut file[total..]);
        if n < 0 {
            close(fd as i32);
            println!("sh: read error on {}: {}", path, n);
            return 1;
        }
        if n == 0 {
            break;
        }
        total += n as usize;
    }
    close(fd as i32);

    let end = total;
    let mut buf = [0u8; LINE + 1];
    let mut start = 0usize;
    let mut i = 0usize;
    while i <= end {
        if i == end || file[i] == b'\n' {
            let mut line: &[u8] = &file[start..i];
            // CRLF: recorta el '\r' final antes de tokenizar (o "ls\r" no casaría).
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.len() > LINE {
                // Nunca ejecutar un comando cortado: la línea se reporta y se salta.
                println!("sh: {}: line longer than {} bytes, skipped", path, LINE);
            } else {
                let m = line.len();
                buf[..m].copy_from_slice(line);
                let (s, e) = trim(&buf, 0, m);
                // Salta vacías y comentarios; run_line hace el resto (incluido `;`), y
                // su `false` (un `exit` en el script) detiene el script, igual que en
                // interactivo.
                if s < e && buf[s] != b'#' && !run_line(&mut buf, s, e) {
                    return 0;
                }
            }
            start = i + 1;
        }
        i += 1;
    }
    0
}

/// Ejecuta una línea ya recortada a `buf[s..e]`. Devuelve false si el usuario pidió
/// salir, true en caso contrario. Interactivo y modo-script comparten esta función.
///
/// DEUDA (`&`, background): no se soporta en SP1. Sin reparentado-a-init, un daemon
/// lanzado con `&` desde `sh /etc/rc` queda huérfano al terminar el script y su entrada
/// de proceso se fuga. El reparentado-a-PID-1 es SP4; el `&` llega entonces.
fn exec_line(buf: &mut [u8], s: usize, e: usize) -> bool {
    if s == e {
        return true;
    }
    if &buf[s..e] == b"exit" {
        return false;
    }
    if &buf[s..e] == b"help" {
        print_help();
        return true;
    }
    if &buf[s..e] == b"clear" {
        // Limpia pantalla (ANSI) + cursor al origen, igual que el builtin del kernel.
        print!("\x1b[2J\x1b[H");
        return true;
    }
    // sudo es un no-op: EspressoOS no tiene separación de privilegios (README §5). Se
    // ejecuta el resto de la línea como si el prefijo no estuviera.
    if buf[s..e].starts_with(b"sudo ") {
        let (ss, se) = trim(buf, s + 5, e);
        if ss < se {
            return exec_line(buf, ss, se);
        }
        return true;
    }
    // cd/pwd son builtins obligatoriamente: un hijo spawneado no puede cambiar el cwd
    // de este proceso.
    if &buf[s..e] == b"pwd" || buf[s..e].starts_with(b"pwd ") {
        builtin_pwd();
        return true;
    }
    if &buf[s..e] == b"cd" || buf[s..e].starts_with(b"cd ") {
        builtin_cd(&buf[s..e]);
        return true;
    }
    // Pipeline de N etapas, partiendo por `|` top-level (`echo "a|b"` es una etapa).
    let mut stages = [(0usize, 0usize); MAX_STAGES];
    let mut nst = 0usize;
    let mut a = s;
    loop {
        let cut = find_top(buf, a, e, b'|');
        let b = cut.unwrap_or(e);
        if nst >= MAX_STAGES {
            println!("sh: too many pipeline stages (max {})", MAX_STAGES);
            return true;
        }
        stages[nst] = (a, b);
        nst += 1;
        match cut {
            Some(k) => a = k + 1,
            None => break,
        }
    }
    if nst == 1 {
        run_single(buf, stages[0]);
    } else {
        run_pipeline_n(buf, &stages[..nst]);
    }
    true
}

/// Ejecuta una línea que puede tener varios comandos separados por `;`, en orden.
/// Devuelve false si algún comando fue `exit`.
///
/// `;` se reconoce aquí (partiendo la línea), no en el tokenizador, por la misma razón
/// que en el shell del kernel: partir la cadena cruda cortaría `echo "a;b"` por la
/// mitad, así que las comillas lo protegen. Un `;` de sobra o final es inofensivo (el
/// segmento vacío se salta). NO hay `&&`/`||`/`&` -- sólo el separador secuencial, que
/// es la paridad que el shell del kernel ya tiene.
fn run_line(buf: &mut [u8], s: usize, e: usize) -> bool {
    let mut seg_start = s;
    let mut i = s;
    let mut quote: u8 = 0; // 0 = fuera de comillas; si no, b'\'' o b'"'
    while i < e {
        let c = buf[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'\'' || c == b'"' {
            quote = c;
        } else if c == b';' {
            let (ss, se) = trim(buf, seg_start, i);
            // exec_line muta buf[ss..se] (tokenize mete NULs), pero eso queda detrás de
            // `i`; el resto de la línea (los segmentos por venir) no se toca.
            if ss < se && !exec_line(buf, ss, se) {
                return false;
            }
            seg_start = i + 1;
        }
        i += 1;
    }
    let (ss, se) = trim(buf, seg_start, e);
    if ss < se {
        return exec_line(buf, ss, se);
    }
    true
}

fn builtin_pwd() {
    let mut cwd = [0u8; PATH_MAX];
    match getcwd_str(&mut cwd) {
        Ok(s) => println!("{}", s),
        Err(n) => println!("pwd: error {}", n),
    }
}

/// Prompt con el cwd, como el shell del kernel ('/' se muestra como '~'). Efecto
/// lateral valioso: `getcwd` se verifica en CADA interacción, gratis.
fn print_prompt() {
    let mut cwd = [0u8; PATH_MAX];
    match getcwd_str(&mut cwd) {
        Ok("/") => print!("EspressoOS:~$ "),
        Ok(s) => print!("EspressoOS:{}$ ", s),
        Err(_) => print!("EspressoOS:?$ "),
    }
}

fn builtin_cd(line: &[u8]) {
    // El argumento tras "cd", recortado. Vacío -> "/" (no hay $HOME en este sistema).
    let target = core::str::from_utf8(&line[2..]).unwrap_or("").trim();
    let target = if target.is_empty() { "/" } else { target };
    let r = chdir(target);
    if r < 0 {
        // Distingue no-existe de no-es-directorio: el mensaje único "no such directory"
        // mentía sobre un fichero que SÍ existe (p. ej. `cd /etc/rc`). Errnos del kernel:
        // ENOENT=-2, ENOTDIR=-20.
        let reason = match r {
            -2 => "no such file or directory",
            -20 => "not a directory",
            _ => "cannot change directory",
        };
        println!("cd: {}: {}", target, reason);
    }
}

/// Reads one line into `buf`, echoing as it goes. Returns its length.
///
/// Isomorfo con el lector del shell del kernel (`shell/mod.rs`): backspace/DEL borran
/// el último carácter y **nunca** pasan del inicio del input -- por eso no se puede
/// comer el prompt. Sólo se aceptan imprimibles `0x20..0x7f`; el resto de controles
/// (flechas, Ctrl-*, etc.) se ignoran en vez de ensuciar el buffer y la pantalla.
const HISTORY_MAX: usize = 16;
static mut HISTORY_BUF: [[u8; LINE]; HISTORY_MAX] = [[0; LINE]; HISTORY_MAX];
static mut HISTORY_LENS: [usize; HISTORY_MAX] = [0; HISTORY_MAX];
static mut HISTORY_COUNT: usize = 0;

fn history_add(line: &[u8]) {
    if line.is_empty() {
        return;
    }
    unsafe {
        if HISTORY_COUNT > 0 {
            let last_idx = (HISTORY_COUNT - 1) % HISTORY_MAX;
            let last_len = HISTORY_LENS[last_idx];
            if &HISTORY_BUF[last_idx][..last_len] == line {
                return;
            }
        }
        let idx = HISTORY_COUNT % HISTORY_MAX;
        let len = line.len().min(LINE);
        HISTORY_BUF[idx][..len].copy_from_slice(&line[..len]);
        HISTORY_LENS[idx] = len;
        HISTORY_COUNT += 1;
    }
}

fn itoa<'a>(mut n: usize, buf: &'a mut [u8; 16]) -> &'a str {
    if n == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap_or("0");
    }
    let mut len = 0;
    let mut tmp = [0u8; 16];
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

fn redraw_line(buf: &[u8], len: usize, pos: usize) {
    let mut num_buf = [0u8; 16];
    let _ = libc::write(1, b"\r");
    print_prompt();
    if len > 0 {
        let _ = libc::write(1, &buf[..len]);
    }
    let _ = libc::write(1, b"\x1b[K");
    if pos < len {
        let back = len - pos;
        let back_str = itoa(back, &mut num_buf);
        let _ = libc::write(1, b"\x1b[");
        let _ = libc::write(1, back_str.as_bytes());
        let _ = libc::write(1, b"D");
    }
}

fn try_read_byte() -> Option<u8> {
    let mut c = [0u8; 1];
    let start = libc::uptime_ms();
    loop {
        if read(0, &mut c) > 0 {
            return Some(c[0]);
        }
        if libc::uptime_ms().saturating_sub(start) > 20 {
            return None;
        }
        yield_now();
    }
}

const BUILTINS: &[&str] = &["help", "clear", "exit", "cd", "pwd", "sudo"];

fn handle_tab_completion(buf: &mut [u8], len: &mut usize, pos: &mut usize) {
    let mut bin_buf = [0u8; 1024];
    let mut dir_buf = [0u8; 1024];

    let word_start = {
        let mut idx = *pos;
        while idx > 0 && buf[idx - 1] != b' ' {
            idx -= 1;
        }
        idx
    };

    let prefix_bytes = &buf[word_start..*pos];
    let prefix = match core::str::from_utf8(prefix_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut matches = [[""; 2]; 32];
    let mut match_count = 0usize;

    if word_start == 0 && !prefix.contains('/') {
        for &b in BUILTINS {
            if b.starts_with(prefix) && match_count < 32 {
                matches[match_count] = [b, " "];
                match_count += 1;
            }
        }
        let n = readdir("/bin", &mut bin_buf);
        if n > 0 {
            let limit = n as usize;
            let mut p = 0;
            while p + 11 <= limit {
                let name_len = u16::from_le_bytes([bin_buf[p + 9], bin_buf[p + 10]]) as usize;
                p += 11;
                if p + name_len > limit {
                    break;
                }
                if let Ok(name) = core::str::from_utf8(&bin_buf[p..p + name_len]) {
                    if name.starts_with(prefix) && match_count < 32 {
                        let mut exists = false;
                        for i in 0..match_count {
                            if matches[i][0] == name {
                                exists = true;
                                break;
                            }
                        }
                        if !exists {
                            matches[match_count] = [name, " "];
                            match_count += 1;
                        }
                    }
                }
                p += name_len;
            }
        }
    } else {
        let (dir_path, file_prefix) = match prefix.rfind('/') {
            Some(slash_pos) => (&prefix[..slash_pos + 1], &prefix[slash_pos + 1..]),
            None => (".", prefix),
        };
        let target_dir = if dir_path.is_empty() { "." } else { dir_path };
        let n = readdir(target_dir, &mut dir_buf);
        if n > 0 {
            let limit = n as usize;
            let mut p = 0;
            while p + 11 <= limit {
                let kind = dir_buf[p + 8];
                let name_len = u16::from_le_bytes([dir_buf[p + 9], dir_buf[p + 10]]) as usize;
                p += 11;
                if p + name_len > limit {
                    break;
                }
                if let Ok(name) = core::str::from_utf8(&dir_buf[p..p + name_len]) {
                    if name != "." && name != ".." && name.starts_with(file_prefix) && match_count < 32 {
                        let suffix = if kind == 1 { "/" } else { " " };
                        matches[match_count] = [name, suffix];
                        match_count += 1;
                    }
                }
                p += name_len;
            }
        }
    }

    if match_count == 1 {
        let name = matches[0][0];
        let suffix = matches[0][1];

        let file_sub_prefix = if word_start != 0 && prefix.contains('/') {
            if let Some(slash_pos) = prefix.rfind('/') {
                &prefix[slash_pos + 1..]
            } else {
                prefix
            }
        } else {
            prefix
        };

        if name.len() >= file_sub_prefix.len() {
            let insert_str = &name[file_sub_prefix.len()..];
            let comp_bytes = insert_str.as_bytes();
            let suff_bytes = suffix.as_bytes();
            let total_add = comp_bytes.len() + suff_bytes.len();

            if *len + total_add <= buf.len() {
                buf.copy_within(*pos..*len, *pos + total_add);
                buf[*pos..*pos + comp_bytes.len()].copy_from_slice(comp_bytes);
                buf[*pos + comp_bytes.len()..*pos + total_add].copy_from_slice(suff_bytes);
                *len += total_add;
                *pos += total_add;
                redraw_line(buf, *len, *pos);
            }
        }
    } else if match_count > 1 {
        println!("");
        for i in 0..match_count {
            print!("{}  ", matches[i][0]);
        }
        println!("");
        redraw_line(buf, *len, *pos);
    }
}

/// Reads one line into `buf`, with editing, history, tab completion, and control shortcuts.
fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    let mut pos = 0usize;

    unsafe {
        let mut history_idx = HISTORY_COUNT;
        let mut draft_buf = [0u8; LINE];
        let mut draft_len = 0usize;

        loop {
            let mut c = [0u8; 1];
            if read(0, &mut c) <= 0 {
                yield_now();
                continue;
            }
            let ch = c[0];

            match ch {
                b'\n' | b'\r' => {
                    println!("");
                    return len;
                }
                0x08 | 0x7f => { // Backspace / DEL
                    if pos > 0 {
                        buf.copy_within(pos..len, pos - 1);
                        len -= 1;
                        pos -= 1;
                        redraw_line(buf, len, pos);
                    }
                }
                0x09 => { // Tab completion
                    handle_tab_completion(buf, &mut len, &mut pos);
                }
                0x01 => { // Ctrl+A (Home)
                    pos = 0;
                    redraw_line(buf, len, pos);
                }
                0x05 => { // Ctrl+E (End)
                    pos = len;
                    redraw_line(buf, len, pos);
                }
                0x15 => { // Ctrl+U (Delete line before cursor)
                    if pos > 0 {
                        buf.copy_within(pos..len, 0);
                        len -= pos;
                        pos = 0;
                        redraw_line(buf, len, pos);
                    }
                }
                0x0b => { // Ctrl+K (Delete line after cursor)
                    if pos < len {
                        len = pos;
                        redraw_line(buf, len, pos);
                    }
                }
                0x17 => { // Ctrl+W (Delete word before cursor)
                    if pos > 0 {
                        let mut wstart = pos;
                        while wstart > 0 && buf[wstart - 1] == b' ' {
                            wstart -= 1;
                        }
                        while wstart > 0 && buf[wstart - 1] != b' ' {
                            wstart -= 1;
                        }
                        let diff = pos - wstart;
                        buf.copy_within(pos..len, wstart);
                        len -= diff;
                        pos = wstart;
                        redraw_line(buf, len, pos);
                    }
                }
                0x0c => { // Ctrl+L (Clear screen)
                    let _ = libc::write(1, b"\x1b[2J\x1b[H");
                    redraw_line(buf, len, pos);
                }
                0x04 => { // Ctrl+D
                    if len == 0 {
                        println!("exit");
                        return usize::MAX;
                    } else if pos < len {
                        buf.copy_within(pos + 1..len, pos);
                        len -= 1;
                        redraw_line(buf, len, pos);
                    }
                }
                0x03 => { // Ctrl+C
                    println!("^C");
                    len = 0;
                    pos = 0;
                    print_prompt();
                }
                0x1b => { // ANSI Escape Sequence
                    if let Some(b'[') = try_read_byte() {
                        if let Some(code) = try_read_byte() {
                            match code {
                                b'A' => { // Up Arrow (History back)
                                    if HISTORY_COUNT > 0 && history_idx > 0 {
                                        let min_idx = if HISTORY_COUNT > HISTORY_MAX { HISTORY_COUNT - HISTORY_MAX } else { 0 };
                                        if history_idx > min_idx {
                                            if history_idx == HISTORY_COUNT {
                                                draft_buf[..len].copy_from_slice(&buf[..len]);
                                                draft_len = len;
                                            }
                                            history_idx -= 1;
                                            let h_idx = history_idx % HISTORY_MAX;
                                            let h_len = HISTORY_LENS[h_idx];
                                            buf[..h_len].copy_from_slice(&HISTORY_BUF[h_idx][..h_len]);
                                            len = h_len;
                                            pos = h_len;
                                            redraw_line(buf, len, pos);
                                        }
                                    }
                                }
                                b'B' => { // Down Arrow (History forward)
                                    if history_idx < HISTORY_COUNT {
                                        history_idx += 1;
                                        if history_idx == HISTORY_COUNT {
                                            buf[..draft_len].copy_from_slice(&draft_buf[..draft_len]);
                                            len = draft_len;
                                            pos = draft_len;
                                        } else {
                                            let h_idx = history_idx % HISTORY_MAX;
                                            let h_len = HISTORY_LENS[h_idx];
                                            buf[..h_len].copy_from_slice(&HISTORY_BUF[h_idx][..h_len]);
                                            len = h_len;
                                            pos = h_len;
                                        }
                                        redraw_line(buf, len, pos);
                                    }
                                }
                                b'C' => { // Right Arrow
                                    if pos < len {
                                        pos += 1;
                                        redraw_line(buf, len, pos);
                                    }
                                }
                                b'D' => { // Left Arrow
                                    if pos > 0 {
                                        pos -= 1;
                                        redraw_line(buf, len, pos);
                                    }
                                }
                                b'H' => { // Home
                                    pos = 0;
                                    redraw_line(buf, len, pos);
                                }
                                b'F' => { // End
                                    pos = len;
                                    redraw_line(buf, len, pos);
                                }
                                b'1' => { // Home (ESC[1~)
                                    let _ = try_read_byte();
                                    pos = 0;
                                    redraw_line(buf, len, pos);
                                }
                                b'4' => { // End (ESC[4~)
                                    let _ = try_read_byte();
                                    pos = len;
                                    redraw_line(buf, len, pos);
                                }
                                b'3' => { // Delete (ESC[3~)
                                    let _ = try_read_byte();
                                    if pos < len {
                                        buf.copy_within(pos + 1..len, pos);
                                        len -= 1;
                                        redraw_line(buf, len, pos);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ if (0x20..0x7f).contains(&ch) => {
                    if len < buf.len() {
                        buf.copy_within(pos..len, pos + 1);
                        buf[pos] = ch;
                        len += 1;
                        pos += 1;
                        redraw_line(buf, len, pos);
                    }
                }
                _ => {}
            }
        }
    }
}

/// `start..end` with leading and trailing blanks removed.
fn trim(buf: &[u8], mut start: usize, mut end: usize) -> (usize, usize) {
    while start < end && (buf[start] == b' ' || buf[start] == b'\t') {
        start += 1;
    }
    while end > start && (buf[end - 1] == b' ' || buf[end - 1] == b'\t') {
        end -= 1;
    }
    (start, end)
}

/// Splits `buf[start..end]` into NUL-terminated tokens in place and fills `argv` with
/// pointers to them, NULL-terminated. Returns the token count.
///
/// The separators become the terminators, so each token ends up a C string pointing
/// into `buf` -- nothing is copied and nothing is allocated, which matters because
/// there is no allocator here. The kernel copies the strings out during `spawn`, so
/// they only have to survive that one call.
///
/// `buf[end]` is overwritten with the last token's NUL. That is why the line buffer
/// is one byte longer than the longest line it accepts.
///
/// None means the line has more tokens than `argv` can hold.
fn tokenize(
    buf: &mut [u8],
    start: usize,
    end: usize,
    argv: &mut [*const u8; MAX_ARGV],
) -> Option<usize> {
    // Respeta comillas simples y dobles: un span entre comillas es UN token y las
    // comillas se eliminan (necesario para `wifi connect "Familia beltre"`). Quitar
    // comillas solo ENCOGE, así que la compactación in-place es segura: el cursor de
    // escritura `write` nunca adelanta al de lectura `read`, así que nunca pisa bytes
    // sin leer. Los tokens quedan NUL-separados dentro de [start..write).
    let mut offs = [0usize; MAX_ARGV];
    let mut n = 0;
    let mut read = start;
    let mut write = start;
    while read < end {
        while read < end && buf[read] == b' ' {
            read += 1;
        }
        if read >= end {
            break;
        }
        // argv[n] takes the token, so argv[n + 1] has to be free for the NULL.
        if n + 1 >= MAX_ARGV {
            return None;
        }
        offs[n] = write;
        n += 1;
        let mut quote: u8 = 0;
        while read < end {
            let c = buf[read];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                } else {
                    buf[write] = c;
                    write += 1;
                }
                read += 1;
            } else if c == b'"' || c == b'\'' {
                quote = c;
                read += 1;
            } else if c == b' ' {
                break;
            } else {
                buf[write] = c;
                write += 1;
                read += 1;
            }
        }
        buf[write] = 0;
        write += 1;
        // El token terminó en un espacio en `read` (o en `end`). Consúmelo: si
        // write==read (sin comillas), el NUL de arriba PISÓ ese espacio, así que el
        // skip de espacios del tope lo vería como 0 y se atascaría leyendo un token
        // vacío. Avanzar `read` aquí lo evita; el skip del tope cubre espacios extra.
        if read < end {
            read += 1;
        }
    }

    // Pointers are taken only once every terminator is in, so nothing derived here
    // is invalidated by a later write through the slice.
    let base = buf.as_ptr();
    for k in 0..n {
        argv[k] = unsafe { base.add(offs[k]) };
    }
    argv[n] = core::ptr::null();
    Some(n)
}

/// Primera aparición top-level (fuera de comillas) de `delim` en `buf[from..to]`.
fn find_top(buf: &[u8], from: usize, to: usize, delim: u8) -> Option<usize> {
    let mut i = from;
    let mut quote: u8 = 0;
    while i < to {
        let c = buf[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'\'' || c == b'"' {
            quote = c;
        } else if c == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Tokenizes a command and resolves its program, reporting why if it cannot.
///
/// Returns the token count; `argv` is left NULL-terminated and ready for `spawn`.
fn prepare<'a>(
    buf: &mut [u8],
    range: (usize, usize),
    argv: &mut [*const u8; MAX_ARGV],
    path_buf: &'a mut [u8],
) -> Option<&'a str> {
    match tokenize(buf, range.0, range.1, argv) {
        None => {
            println!("Too many arguments (max {})", MAX_ARGV - 1);
            return None;
        }
        Some(0) => {
            println!("Empty command");
            return None;
        }
        Some(_) => {}
    }

    // argv[0] is the name as typed; the path is where it was found. They differ for
    // anything run by bare name, and the child is meant to see the former.
    let path = resolve_path(unsafe { libc::arg(argv.as_ptr(), 0) }, path_buf);
    if path.is_empty() {
        println!("Invalid command path");
        return None;
    }
    Some(path)
}

/// Separa el rango de un stage en (rango_comando, redirección opcional). La redirección
/// es el primer `>` (o `>>` = append) top-level y su fichero destino (un token).
fn parse_redirect(
    buf: &[u8],
    a: usize,
    b: usize,
) -> ((usize, usize), Option<(usize, usize, bool)>) {
    match find_top(buf, a, b, b'>') {
        None => ((a, b), None),
        Some(i) => {
            let append = i + 1 < b && buf[i + 1] == b'>';
            let after = if append { i + 2 } else { i + 1 };
            let (ts, te0) = trim(buf, after, b);
            // El destino es un solo token: hasta el siguiente espacio top-level.
            let te = find_top(buf, ts, te0, b' ').unwrap_or(te0);
            ((a, i), Some((ts, te, append)))
        }
    }
}

/// Abre el fichero destino de una redirección y devuelve su fd (<0 en error). Recorta
/// comillas envolventes. `open` toma ptr+len, así que no hace falta NUL-terminar.
fn redir_open(buf: &[u8], ts: usize, te: usize, append: bool) -> isize {
    let (mut a, mut b) = (ts, te);
    if b > a + 1 && (buf[a] == b'"' || buf[a] == b'\'') && buf[b - 1] == buf[a] {
        a += 1;
        b -= 1;
    }
    let path = match core::str::from_utf8(&buf[a..b]) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    let flags = if append {
        O_WRONLY | O_CREATE | O_APPEND
    } else {
        O_WRONLY | O_CREATE | O_TRUNC
    };
    open(path, flags)
}

fn run_single(buf: &mut [u8], range: (usize, usize)) {
    let (mut cs, mut ce) = trim(buf, range.0, range.1);
    let mut is_bg = false;
    if ce > cs && buf[ce - 1] == b'&' {
        is_bg = true;
        ce -= 1;
        let trimmed = trim(buf, cs, ce);
        cs = trimmed.0;
        ce = trimmed.1;
    }

    let (cmd, redir) = parse_redirect(buf, cs, ce);

    // La redirección se abre ANTES de tokenizar el comando: `redir_open` lee bytes del
    // destino, que están detrás del `>` y no los toca la compactación de `tokenize`.
    let mut rfd = -1i32;
    if let Some((ts, te, append)) = redir {
        let fd = redir_open(buf, ts, te, append);
        if fd < 0 {
            println!("sh: cannot open redirection target");
            return;
        }
        rfd = fd as i32;
    }

    let mut argv = [core::ptr::null(); MAX_ARGV];
    let mut path_buf = [0u8; LINE + 8];
    let path = match prepare(buf, cmd, &mut argv, &mut path_buf) {
        Some(p) => p,
        None => {
            if rfd >= 0 {
                close(rfd);
            }
            return;
        }
    };

    // Redirección = dup2 sobre fd 1, guardando el original en fd 12. El hijo nunca sabe
    // que se movió (misma semántica documentada en el README §6).
    let mut saved = -1i32;
    if rfd >= 0 {
        saved = dup2(1, 12) as i32;
        dup2(rfd, 1);
        close(rfd);
    }

    let pid = spawn(path, argv.as_ptr());
    if pid < 0 {
        println!("Error executing: {}", path);
    } else if is_bg {
        println!("[1] {}", pid);
    } else {
        let mut status = 0;
        let _ = wait(&mut status);
    }

    if saved >= 0 {
        dup2(saved, 1);
        close(saved);
    }
}

/// Pipeline de N etapas (`a | b | c | ...`). Crea N-1 pipes y encadena stdin/stdout de
/// cada etapa; una etapa con redirección `>` propia manda su stdout al fichero en vez del
/// pipe (gana al pipe, como el shell del kernel — README §6). Prepara y lanza cada etapa
/// incrementalmente con la MISMA disciplina de fds que la versión de 2 etapas: el
/// write-end de cada pipe sale de ESTA tabla (lo reasigna el `dup2` de la etapa siguiente)
/// antes de lanzar la etapa que lo lee, así ninguna etapa hereda un escritor del pipe que
/// va a leer y se cuelga esperando un EOF que no llega.
fn run_pipeline_n(buf: &mut [u8], stages: &[(usize, usize)]) {
    let n = stages.len();
    let saved_out = dup2(1, 10) as i32;
    let saved_in = dup2(0, 11) as i32;

    let mut prev_read = -1i32; // read-end del pipe anterior = stdin de esta etapa
    let mut nspawned = 0usize;

    for i in 0..n {
        let (a, b) = stages[i];
        let (cmd, redir) = parse_redirect(buf, a, b);

        let mut rfd = -1i32;
        if let Some((ts, te, append)) = redir {
            let fd = redir_open(buf, ts, te, append);
            if fd >= 0 {
                rfd = fd as i32;
            }
        }

        let mut argv = [core::ptr::null(); MAX_ARGV];
        let mut path_buf = [0u8; LINE + 8];
        let path = match prepare(buf, cmd, &mut argv, &mut path_buf) {
            Some(p) => p,
            None => {
                if rfd >= 0 {
                    close(rfd);
                }
                break;
            }
        };

        // Pipe hacia la siguiente etapa (salvo la última).
        let mut next_read = -1i32;
        let mut this_write = -1i32;
        if i + 1 < n {
            let mut pp = [0i32; 2];
            if pipe(&mut pp) < 0 {
                println!("sh: pipe error");
                if rfd >= 0 {
                    close(rfd);
                }
                break;
            }
            next_read = pp[0];
            this_write = pp[1];
        }

        // stdin de esta etapa: del pipe anterior, o el stdin original en la primera.
        if prev_read >= 0 {
            dup2(prev_read, 0);
        } else {
            dup2(saved_in, 0);
        }
        // stdout: redirección propia (gana) > pipe siguiente > stdout original.
        if rfd >= 0 {
            dup2(rfd, 1);
        } else if this_write >= 0 {
            dup2(this_write, 1);
        } else {
            dup2(saved_out, 1);
        }

        let pid = spawn(path, argv.as_ptr());
        if pid >= 0 {
            nspawned += 1;
        }

        // El hijo ya clonó la tabla; suelta en ESTA los fds que no necesita.
        if rfd >= 0 {
            close(rfd);
        }
        if prev_read >= 0 {
            close(prev_read);
        }
        if this_write >= 0 {
            close(this_write);
        }
        prev_read = next_read;
    }
    if prev_read >= 0 {
        close(prev_read);
    }

    // Restaura stdio y espera a las etapas lanzadas.
    dup2(saved_out, 1);
    dup2(saved_in, 0);
    close(saved_out);
    close(saved_in);
    for _ in 0..nspawned {
        let mut status = 0;
        let _ = wait(&mut status);
    }
}

fn print_help() {
    println!("EspressoOS userland shell -- built-in commands & features:");
    println!("  help                 show this help message");
    println!("  clear                clear the screen");
    println!("  cd [PATH]            change working directory");
    println!("  pwd                  print working directory");
    println!("  sudo <cmd>           run command with root privilege shim");
    println!("  exit                 exit the shell");
    println!("  <cmd> ; <cmd>        run commands sequentially");
    println!("  <cmd> &              run command in background");
    println!("  <cmd> | <cmd>        pipe output across commands (up to 8 stages)");
    println!("  <cmd> > <file>       redirect stdout to file (truncate)");
    println!("  <cmd> >> <file>      redirect stdout to file (append)");
    println!("  \"text\" / 'text'      quote-aware argument parsing");
    println!("");
    println!("Line Editing Shortcuts:");
    println!("  Up / Down (Arrow)    navigate command history");
    println!("  Left / Right (Arrow) move cursor left / right");
    println!("  Home / End           jump to start / end of line");
    println!("  Tab                  auto-complete command or path");
    println!("  Ctrl+A / Ctrl+E      jump to start / end of line");
    println!("  Ctrl+U / Ctrl+K      delete line before / after cursor");
    println!("  Ctrl+W               delete word before cursor");
    println!("  Ctrl+L               clear screen and redraw line");
    println!("  Ctrl+C               abort current draft line");
    println!("  Ctrl+D               exit shell (on empty line)");
    println!("");
    println!("Programs in /bin (run by name, e.g. 'ls' or '/bin/ls'):");
    let mut buf = [0u8; 1024];
    let n = readdir("/bin", &mut buf);
    if n < 0 {
        println!("  (could not read /bin)");
        return;
    }
    let mut pos = 0;
    let limit = n as usize;
    while pos < limit {
        if pos + 11 > limit {
            break;
        }
        let name_len = u16::from_le_bytes([buf[pos + 9], buf[pos + 10]]) as usize;
        pos += 11;
        if pos + name_len > limit {
            break;
        }
        if let Ok(name) = core::str::from_utf8(&buf[pos..pos + name_len]) {
            println!("  {}", name);
        }
        pos += name_len;
    }
}

/// Where to look for `cmd`: /bin if it is a bare name, otherwise wherever it says.
///
/// This is PATH search and nothing else. The /bin prefix must stay -- `ls` means the
/// program, and dropping it because "the VFS resolves paths now" would send `ls` after
/// a `cd /tmp` looking for /tmp/ls. But it applies only to a bare NAME.
///
/// The test is `contains('/')`, not `starts_with('/')`, which is what the kernel shell
/// has always used. With starts_with, `./hello` was not absolute, so it got the prefix:
/// "/bin/./hello", which normalize collapses to "/bin/hello". Typing ./hello in /tmp
/// ran a different program with the same name and said nothing, or reported "not found"
/// about a path never typed. /tmp/hello was never consulted.
fn resolve_path<'a>(cmd: &str, out_buf: &'a mut [u8]) -> &'a str {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return "";
    }
    if cmd.contains('/') {
        let cmd_bytes = cmd.as_bytes();
        if cmd_bytes.len() < out_buf.len() {
            out_buf[..cmd_bytes.len()].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&out_buf[..cmd_bytes.len()]).unwrap_or("")
        } else {
            ""
        }
    } else {
        let prefix = b"/bin/";
        let cmd_bytes = cmd.as_bytes();
        if prefix.len() + cmd_bytes.len() < out_buf.len() {
            out_buf[..prefix.len()].copy_from_slice(prefix);
            out_buf[prefix.len()..prefix.len() + cmd_bytes.len()].copy_from_slice(cmd_bytes);
            core::str::from_utf8(&out_buf[..prefix.len() + cmd_bytes.len()]).unwrap_or("")
        } else {
            ""
        }
    }
}
