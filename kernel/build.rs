










use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::Command;

const APPS: &[(&str, u32)] = &[
    ("init", 0),
    ("sh", 1),
    ("cat", 2),
    ("echo", 2),
    ("ls", 2),
    ("ota", 2),
    ("ping", 2),
    ("sntp", 2),
    ("netstat", 2),
    ("httpd", 2),
];

fn main() {
    println!("cargo:rerun-if-changed=../userland/apps/src");
    println!("cargo:rerun-if-changed=../userland/libc/src");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let uland = PathBuf::from(&manifest)
        .parent()
        .unwrap()
        .join("userland");
    let slots_dir = uland.join(".slots");
    fs::create_dir_all(&slots_dir).unwrap();


    let mut seen = HashSet::new();
    for &(_, slot) in APPS {
        if !seen.insert(slot) {
            continue;
        }
        let text = 0x4280_0000u32 + slot * 0x1_0000;
        let data = 0x3c17_0000u32 + slot * 0x1_0000;
        let script = format!(
            "ENTRY(_start)\n\
MEMORY {{\n\
\x20 ITEXT (rx) : ORIGIN = 0x{text:08x}, LENGTH = 64K\n\
\x20 UDATA (rw) : ORIGIN = 0x{data:08x}, LENGTH = 64K\n\
}}\n\
SECTIONS {{\n\
\x20 .text : {{ *(.literal._start) *(.text._start) *(.literal .literal.*) *(.text .text.*) }} > ITEXT\n\
\x20 .rodata : {{ *(.rodata .rodata.*) }} > UDATA\n\
\x20 .data : {{ *(.data .data.*) }} > UDATA\n\
\x20 .bss : {{ *(.bss .bss.*) }} > UDATA\n\
}}\n"
        );
        fs::write(slots_dir.join(format!("user_s{slot}.x")), script).unwrap();
    }



    for &(name, slot) in APPS {
        let rustflags = format!(
            "-C link-arg=-nostartfiles -C force-frame-pointers -C link-arg=-L.slots -C link-arg=-Tuser_s{slot}.x"
        );
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
    let mut f = File::create(Path::new(&out_dir).join("userland_bin.rs")).unwrap();
    writeln!(f, "pub const USERLAND_BINARIES: &[(&str, &[u8])] = &[").unwrap();
    for &(name, _) in APPS {
        let p = target.join(name);
        println!("cargo:rerun-if-changed={}", p.display());
        let ps = p.to_str().unwrap().replace('\\', "/");
        writeln!(f, "    (\"{name}\", include_bytes!(\"{ps}\")),").unwrap();
    }
    writeln!(f, "];").unwrap();
}
