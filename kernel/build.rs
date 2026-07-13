// build.rs — Empotra los binarios de userland (userland/dist/*.elf) en el kernel.
//
// Genera $OUT_DIR/userland_bin.rs con una tabla `USERLAND_BINARIES` de
// (nombre, &[u8]) usando include_bytes!. Si userland/dist/ no existe o esta
// vacio (no se corrio tools/build-userland.ps1), la tabla queda vacia y el
// kernel arranca con su shell interna (fallback). Asi el build NUNCA falla por
// falta de userland.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dist = PathBuf::from(&manifest)
        .parent()
        .expect("kernel parent")
        .join("userland")
        .join("dist");

    println!("cargo:rerun-if-changed={}", dist.display());

    let mut code = String::from("pub static USERLAND_BINARIES: &[(&str, &[u8])] = &[\n");

    if let Ok(rd) = std::fs::read_dir(&dist) {
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "elf").unwrap_or(false))
            .collect();
        files.sort();
        for p in files {
            let name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let path_str = p.to_string_lossy().replace('\\', "/");
            println!("cargo:rerun-if-changed={}", p.display());
            writeln!(code, "    ({:?}, include_bytes!({:?})),", name, path_str).unwrap();
        }
    }

    code.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("userland_bin.rs");
    std::fs::write(&out, code).expect("write userland_bin.rs");
}
