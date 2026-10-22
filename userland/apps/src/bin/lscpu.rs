#![no_std]
#![no_main]

use libc::{arg, println};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 {
        let a = unsafe { arg(argv, 1) };
        if a == "-h" || a == "--help" {
            println!("Usage: lscpu");
            println!("Display information about the CPU architecture");
            return 0;
        }
    }

    println!("Architecture:          xtensa");
    println!("CPU op-mode(s):        32-bit");
    println!("Byte Order:            Little Endian");
    println!("CPU(s):                2");
    println!("On-line CPU(s) list:   0,1");
    println!("Model name:            Cadence Tensilica Xtensa LX7");
    println!("CPU max MHz:           240.0000");
    println!("CPU min MHz:           80.0000");
    println!("BogoMIPS:              480.00");
    println!("Flags:                 fpu, windowed_abi, dma, trng, aes, sha, rsa");
    println!("L1d cache:             32 KiB");
    println!("L1i cache:             32 KiB");
    println!("Internal SRAM:         512 KiB (IRAM/DRAM)");
    println!("External PSRAM:        8192 KiB (Octal SPI @ 80-120 MHz)");
    println!("Flash Memory:          16384 KiB (Quad/Octal SPI NOR Flash)");
    println!("Security/PMS:          World Controller / PMS Region Protection");
    0
}
