//! build.rs del bootloader — genera/instala el linker script personalizado.
//!
//! ESQUELETO. Cuando se aborde el bootloader propio, aquí se emitirá un
//! `link.x` con el mapa de memoria del ESP32-S3 (IRAM/DRAM/RTC) y se
//! expondrá al linker vía `cargo:rustc-link-search`.

fn main() {
    // TODO(fase-bootloader): escribir link.x en OUT_DIR y añadirlo al search path.
    // let out = std::env::var("OUT_DIR").unwrap();
    // std::fs::write(format!("{out}/link.x"), LINKER_SCRIPT).unwrap();
    // println!("cargo:rustc-link-search={out}");
    println!("cargo:rerun-if-changed=build.rs");
}
