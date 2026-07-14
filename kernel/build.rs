use std::fs::File;
use std::io::Write as IoWrite;
use std::path::Path;
use std::process::Command;
use std::env;

fn main() {
    println!("cargo:rerun-if-changed=../userland/apps/src");
    println!("cargo:rerun-if-changed=../userland/libc/src");

    // 1. Compilar userland usando cargo
    let rustflags = "-C link-arg=-Tuser.x -C force-frame-pointers -C link-arg=-nostartfiles";
    
    let mut cmd = Command::new("cargo");
    cmd.args(&["build", "--release"]);
    cmd.current_dir("../userland");
    cmd.env("RUSTFLAGS", rustflags);
    
    // Quitar variables que puedan interferir
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    cmd.env_remove("RUSTC_WORKSPACE_WRAPPER");
    
    let status = cmd.status().expect("Fallo al ejecutar cargo build en userland");
    if !status.success() {
        panic!("La compilación de userland falló");
    }

    // 2. Generar el archivo userland_bin.rs en OUT_DIR con rutas absolutas
    let project_root = env::current_dir().unwrap(); // Directorio kernel/
    let workspace_root = project_root.parent().unwrap();
    let userland_target_dir = workspace_root
        .join("userland")
        .join("target")
        .join("xtensa-esp32s3-none-elf")
        .join("release");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("userland_bin.rs");
    let mut f = File::create(&dest_path).unwrap();

    let binaries = &[
        "init",
        "sh",
        "cat",
        "echo",
        "ls",
        "ota",
        "ping",
        "sntp",
        "netstat",
        "httpd",
    ];

    writeln!(f, "pub const USERLAND_BINARIES: &[(&str, &[u8])] = &[").unwrap();
    for name in binaries {
        let binary_path = userland_target_dir.join(name);
        let path_str = binary_path.to_str().unwrap().replace("\\", "/");
        writeln!(
            f,
            "    (\"{}\", include_bytes!(\"{}\")),",
            name, path_str
        ).unwrap();
    }
    writeln!(f, "];").unwrap();
}
