use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::Command;

const APPS: &[&str] = &[
    "init", "sh", "cat", "echo", "ls", "ota", "ping", "sntp", "netstat", "httpd", "sleep", "badptr",
    "cwdtest", "mkdir", "touch", "rm", "write", "wifi", "ip", "nmcli", "ioctltest",
    "uptime", "free", "ps", "smp", "pms", "i2c", "spi",
    "sha256", "power", "ble", "reboot", "tcping", "kill",
];

// Every binary links at the same address and the loader relocates it into whatever
// slot is free (see extract_fixups). Programs used to get a fixed slot each, which
// meant two instances of one program could never coexist -- and with ten programs
// in eight slots, eight of them shared slot 2, so `ls` and `cat` could not run at
// the same time either. That is what blocked pipelines.
//
// These three must agree with mm::psram_exec; they are emitted into userland_bin.rs
// and asserted there, because a linker script that disagrees with the pool would
// overflow into the neighbouring slot at run time and not at build time.
const LINK_TEXT: u32 = 0x4280_0000; // == psram_exec::slot_text_exec(0)
const LINK_DATA: u32 = 0x3c17_0000; // == psram_exec::slot_data(0)
const SLOT_SIZE: u32 = 16 * 1024; // == psram_exec::SLOT_SIZE

// ---------------------------------------------------------------------------
// Fixup extraction
//
// The kernel must be able to load a binary at a slot other than the one it was
// linked for. Xtensa cannot do that the usual way: the LLVM backend rejects PIC
// outright ("PIC relocations is not supported"), so there is no PIE to load.
//
// What saves it is an ISA quirk. Xtensa cannot encode a 32-bit absolute in an
// instruction, so every far reference goes through the literal pool via L32R --
// and the literal pool is DATA. Link with `ld --emit-relocs` and the only thing
// that needs patching to move an image is a handful of data words: 11 for cat,
// 48 for sh. The 879 R_XTENSA_SLOT0_OP relocations across all ten binaries are
// PC-relative and every one targets its own .text, so a uniform text bias leaves
// them correct and the loader skips them entirely. No instruction is decoded.
//
// This runs on the host so the invariants can be asserted where a violation
// fails the build instead of the board.
// ---------------------------------------------------------------------------

const R_XTENSA_32: u32 = 1;
const R_XTENSA_SLOT0_OP: u32 = 20;
const SHT_RELA: u32 = 4;
const SHT_NOBITS: u32 = 8;
const PT_LOAD: u32 = 1;

/// Anything mapped at or above this is on the instruction bus; anything below is
/// data. The two userland regions are far apart, so one compare classifies a word.
const IBUS_MIN: u32 = 0x4200_0000;

fn rd32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn rd16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

struct Sec {
    name: String,
    typ: u32,
    addr: u32,
    off: u32,
    size: u32,
    entsize: u32,
}

fn sections(d: &[u8]) -> Vec<Sec> {
    let shoff = rd32(d, 0x20) as usize;
    let shentsize = rd16(d, 0x2e) as usize;
    let shnum = rd16(d, 0x30) as usize;
    let shstrndx = rd16(d, 0x32) as usize;

    let raw: Vec<(u32, u32, u32, u32, u32, u32)> = (0..shnum)
        .map(|i| {
            let o = shoff + i * shentsize;
            (
                rd32(d, o),
                rd32(d, o + 4),
                rd32(d, o + 12),
                rd32(d, o + 16),
                rd32(d, o + 20),
                rd32(d, o + 36),
            )
        })
        .collect();
    let stro = raw[shstrndx].3 as usize;
    raw.iter()
        .map(|&(nm, typ, addr, off, size, entsize)| {
            let s = stro + nm as usize;
            let e = d[s..].iter().position(|&c| c == 0).unwrap() + s;
            Sec {
                name: String::from_utf8_lossy(&d[s..e]).into_owned(),
                typ,
                addr,
                off,
                size,
                entsize,
            }
        })
        .collect()
}

/// (text ranges, data ranges) from PT_LOAD.
fn seg_ranges(d: &[u8]) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
    let phoff = rd32(d, 0x1c) as usize;
    let phentsize = rd16(d, 0x2a) as usize;
    let phnum = rd16(d, 0x2c) as usize;
    let (mut t, mut da) = (Vec::new(), Vec::new());
    for i in 0..phnum {
        let o = phoff + i * phentsize;
        let (pt, pv, pm) = (rd32(d, o), rd32(d, o + 8), rd32(d, o + 20));
        if pt != PT_LOAD || pm == 0 {
            continue;
        }
        if pv >= IBUS_MIN {
            t.push((pv, pv + pm))
        } else {
            da.push((pv, pv + pm))
        }
    }
    (t, da)
}

fn within(v: u32, rs: &[(u32, u32)]) -> bool {
    rs.iter().any(|&(lo, hi)| v >= lo && v < hi)
}

/// Reads the word an ELF virtual address points at, straight out of the file.
///
/// This is the whole trick, and it is why `S + A` must NOT be used: `ld
/// --emit-relocs` leaves the addend at 0 for section symbols and keeps the fully
/// resolved value in the word instead. 240 of the 273 relocations across the ten
/// binaries have `S + A != word`. The word is right by definition -- these images
/// run today -- so the loader read-modify-writes it and never consults a symbol.
fn word_at(d: &[u8], secs: &[Sec], addr: u32) -> Option<u32> {
    secs.iter().find_map(|s| {
        if s.typ == SHT_NOBITS || s.addr == 0 {
            return None;
        }
        if addr >= s.addr && addr < s.addr + s.size {
            Some(rd32(d, (s.off + (addr - s.addr)) as usize))
        } else {
            None
        }
    })
}

/// One u32 per fixup:
///   bits 2.. : byte offset of the word inside its own region
///   bit 1    : 0 = the word lives in the text image, 1 = in the data image
///   bit 0    : 0 = add text_bias to it, 1 = add data_bias
///
/// The two low bits are free because every R_XTENSA_32 offset is 4-byte aligned
/// (verified: 0 of 273 misaligned). Both bits are needed and neither is
/// redundant: a `.rodata` function-pointer table is a word living in DATA whose
/// value points at TEXT, and 7 of the 10 binaries have one.
fn extract_fixups(name: &str, d: &[u8]) -> Vec<u32> {
    let secs = sections(d);
    let (text_r, data_r) = seg_ranges(d);
    let tbase = text_r.iter().map(|r| r.0).min().unwrap_or(0);
    let dbase = data_r.iter().map(|r| r.0).min().unwrap_or(0);

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for s in secs.iter().filter(|s| s.typ == SHT_RELA) {
        for i in 0..(s.size / s.entsize) {
            let o = (s.off + i * s.entsize) as usize;
            let (r_off, r_info) = (rd32(d, o), rd32(d, o + 4));
            match r_info & 0xFF {
                // PC-relative, and every target is inside this binary's own .text
                // (verified: 0 of 879 escape), so a uniform text bias keeps it
                // valid. Nothing to do.
                R_XTENSA_SLOT0_OP => continue,
                R_XTENSA_32 => {}
                // NONE is padding ld leaves behind. Anything else means an
                // assumption broke -- fail loudly rather than emit a table that
                // silently misses fixups.
                0 => continue,
                t => panic!(
                    "userland '{name}': unexpected relocation type {t} at 0x{r_off:08x} in {}. \
                     The loader only knows R_XTENSA_32; see the fixup notes in build.rs.",
                    s.name
                ),
            }

            assert!(
                r_off % 4 == 0,
                "userland '{name}': fixup at 0x{r_off:08x} is not 4-byte aligned, so the low bits \
                 are not free and the fixup encoding is invalid"
            );

            let (in_data, base) = if within(r_off, &text_r) {
                (0u32, tbase)
            } else if within(r_off, &data_r) {
                (1u32, dbase)
            } else {
                panic!("userland '{name}': fixup at 0x{r_off:08x} is outside every PT_LOAD");
            };

            let w = word_at(d, &secs, r_off).unwrap_or_else(|| {
                panic!("userland '{name}': fixup at 0x{r_off:08x} has no bytes in the file")
            });

            let bias = if within(w, &text_r) {
                0u32
            } else if within(w, &data_r) {
                1u32
            } else {
                panic!(
                    "userland '{name}': word at 0x{r_off:08x} is 0x{w:08x}, which is in neither the \
                     text nor the data image, so the loader cannot tell which bias to add"
                );
            };

            let off = r_off - base;
            assert!(
                seen.insert((in_data, off)),
                "userland '{name}': two fixups for the same word (region {in_data}, offset 0x{off:x})"
            );
            out.push((off & !3) | (in_data << 1) | bias);
        }
    }
    out.sort_unstable();
    out
}

/// `<elf bytes><fixups u32[]><count u32><magic u32>`, so the loader finds the
/// table by seeking from the end and never touches a section header.
const FIXUP_MAGIC: u32 = 0x4553_5046; // "ESPF"

fn with_fixup_trailer(elf: &[u8], fixups: &[u32]) -> Vec<u8> {
    let mut v = elf.to_vec();
    for f in fixups {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v.extend_from_slice(&(fixups.len() as u32).to_le_bytes());
    v.extend_from_slice(&FIXUP_MAGIC.to_le_bytes());
    v
}

fn main() {
    println!("cargo:rerun-if-changed=../userland/apps/src");
    println!("cargo:rerun-if-changed=../userland/libc/src");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let uland = PathBuf::from(&manifest).parent().unwrap().join("userland");
    let slots_dir = uland.join(".slots");
    fs::create_dir_all(&slots_dir).unwrap();

    // LENGTH is the slot size, not a round number: the linker then refuses a binary
    // that would not fit a slot ("region ITEXT overflowed"), instead of the overflow
    // showing up on the board as a program quietly scribbling on its neighbour.
    let script = format!(
        "ENTRY(_start)\n\
MEMORY {{\n\
\x20 ITEXT (rx) : ORIGIN = 0x{LINK_TEXT:08x}, LENGTH = {slot_k}K\n\
\x20 UDATA (rw) : ORIGIN = 0x{LINK_DATA:08x}, LENGTH = {slot_k}K\n\
}}\n\
SECTIONS {{\n\
\x20 .text : {{ *(.literal._start) *(.text._start) *(.literal .literal.*) *(.text .text.*) }} > ITEXT\n\
\x20 .rodata : {{ *(.rodata .rodata.*) }} > UDATA\n\
\x20 .data : {{ *(.data .data.*) }} > UDATA\n\
\x20 .bss : {{ *(.bss .bss.*) }} > UDATA\n\
}}\n",
        slot_k = SLOT_SIZE / 1024
    );
    // cargo does not treat a linker script as an input to anything, so changing the
    // slot geometry regenerates this file and relinks nothing -- every binary would
    // keep the old layout, silently. Deleting the output is not enough either:
    // release/<bin> is a hardlink of a cached deps/<bin>-<hash>, so cargo just
    // re-links it without invoking ld. Bumping the source mtime is what actually
    // forces the relink.
    let script_path = slots_dir.join("espresso.x");
    let script_changed = fs::read_to_string(&script_path).map_or(true, |old| old != script);
    fs::write(&script_path, &script).unwrap();
    if script_changed {
        for &name in APPS {
            let src = uland.join("apps/src/bin").join(format!("{name}.rs"));
            if let Ok(f) = File::options().write(true).open(&src) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
        }
    }

    for &name in APPS {
        // --emit-relocs keeps .rela.* in the fully linked image. That is where the
        // fixups come from; the sections are non-alloc, so they are not in any
        // PT_LOAD and cost nothing but file bytes.
        //
        // The -T path carries a directory component on purpose. Given a bare name,
        // ld resolves it from the current directory before the -L path -- which is
        // how a checked-in userland/user.x silently beat the generated script and
        // linked everything 64 KB away from where the slot pool expects it. With a
        // slash, the file is used as given and never searched for.
        let rustflags = "-C link-arg=-nostartfiles -C force-frame-pointers \
             -C link-arg=-T.slots/espresso.x -C link-arg=-Wl,--emit-relocs"
            .to_string();
        let status = Command::new("cargo")
            .args(["build", "--release", "--bin", name])
            .current_dir(&uland)
            .env("RUSTFLAGS", &rustflags)
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .status()
            .expect("fallo al ejecutar cargo build del userland");
        if !status.success() {
            panic!("la compilación de userland '{name}' falló");
        }
    }

    let target = uland.join("target/xtensa-esp32s3-none-elf/release");
    let out_dir = env::var("OUT_DIR").unwrap();
    let img_dir = Path::new(&out_dir).join("userland_img");
    fs::create_dir_all(&img_dir).unwrap();

    let mut f = File::create(Path::new(&out_dir).join("userland_bin.rs")).unwrap();
    writeln!(
        f,
        "// What the linker script above actually used. mm::psram_exec asserts against\n\
         // these, so a script that drifts from the slot pool fails the build rather\n\
         // than overflowing into the next slot at run time.\n\
         pub const USERLAND_LINK_TEXT: u32 = 0x{LINK_TEXT:08x};\n\
         pub const USERLAND_LINK_DATA: u32 = 0x{LINK_DATA:08x};\n\
         pub const USERLAND_SLOT_SIZE: u32 = {SLOT_SIZE};\n"
    )
    .unwrap();
    writeln!(f, "pub const USERLAND_BINARIES: &[(&str, &[u8])] = &[").unwrap();
    for &name in APPS {
        let p = target.join(name);
        println!("cargo:rerun-if-changed={}", p.display());

        let elf = fs::read(&p).unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", p.display()));
        let fixups = extract_fixups(name, &elf);
        println!(
            "cargo:warning=userland {name}: {} fixups ({} B de tabla)",
            fixups.len(),
            fixups.len() * 4
        );

        let img = img_dir.join(name);
        fs::write(&img, with_fixup_trailer(&elf, &fixups)).unwrap();
        let ps = img.to_str().unwrap().replace('\\', "/");
        writeln!(f, "    (\"{name}\", include_bytes!(\"{ps}\")),").unwrap();
    }
    writeln!(f, "];").unwrap();
}
