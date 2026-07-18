#![no_std]
#![no_main]

use libc::{close, ioctl, open, println};

const O_RDONLY: u32 = 1;
const WLAN_NOP: u32 = 0;
const WLAN_CONNECT: u32 = 1;
const I2C_READ: u32 = 1;
const SPI_TRANSFER: u32 = 0;

const EFAULT: isize = -14;
const EINVAL: isize = -22;

// Puntero sin mapear, igual criterio que badptr/cwdtest.
const BAD_PTR: usize = 0xDEAD_BEEF;

/// Espejo del struct del kernel (drivers::wifi::WlanConnectReq, D-1).
#[repr(C)]
struct ConnectReq {
    ssid_ptr: usize,
    ssid_len: usize,
    pass_ptr: usize,
    pass_len: usize,
}

#[repr(C)]
struct I2cReq {
    addr: usize,
    buf_ptr: usize,
    len: usize,
}

#[repr(C)]
struct SpiReq {
    buf_ptr: usize,
    len: usize,
}

fn check(name: &str, got: isize, want: isize, fails: &mut i32) {
    if got == want {
        println!("[ioctltest] OK   {} -> {}", name, got);
    } else {
        println!("[ioctltest] FAIL {} -> {} (esperado {})", name, got, want);
        *fails += 1;
    }
}

/// Self-test de la frontera del ioctl de /dev/wlan0 (estilo badptr): conoce las respuestas
/// correctas, así que ningún caso puede pasar por accidente. NO conecta ni escanea de
/// verdad (usa WLAN_NOP como control válido) — no altera el estado de la red.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = open("/dev/wlan0", O_RDONLY);
    if fd < 0 {
        println!("[ioctltest] cannot open /dev/wlan0");
        return 1;
    }
    let fd = fd as i32;
    let mut fails = 0;
    let anchor = "test";

    // Control: cmd válido sin efectos -> 0.
    check("nop (cmd valido)", ioctl(fd, WLAN_NOP, 0), 0, &mut fails);

    // Puntero interno inválido -> EFAULT, y el kernel NO entra en pánico (D-1).
    let bad = ConnectReq {
        ssid_ptr: BAD_PTR,
        ssid_len: 5,
        pass_ptr: anchor.as_ptr() as usize,
        pass_len: 0,
    };
    check(
        "connect(ssid_ptr invalido)",
        ioctl(fd, WLAN_CONNECT, &bad as *const ConnectReq as usize),
        EFAULT,
        &mut fails,
    );

    // SSID de 33 bytes -> rechazo por límite D-2, antes de tocar el puntero.
    let big = ConnectReq {
        ssid_ptr: anchor.as_ptr() as usize,
        ssid_len: 33,
        pass_ptr: anchor.as_ptr() as usize,
        pass_len: 0,
    };
    check(
        "connect(ssid_len=33)",
        ioctl(fd, WLAN_CONNECT, &big as *const ConnectReq as usize),
        EINVAL,
        &mut fails,
    );

    // cmd desconocido -> errno.
    check("ioctl(cmd=99)", ioctl(fd, 99, 0), EINVAL, &mut fails);
    close(fd);

    // --- El molde D-1 generaliza a i2c/spi: misma validación de struct + puntero interno ---
    let anchor = [0u8; 4];
    let i2c = open("/dev/i2c0", O_RDONLY);
    if i2c >= 0 {
        let i2c = i2c as i32;
        let bad = I2cReq { addr: 0x50, buf_ptr: BAD_PTR, len: 4 };
        check(
            "i2c read(buf_ptr invalido)",
            ioctl(i2c, I2C_READ, &bad as *const I2cReq as usize),
            EFAULT,
            &mut fails,
        );
        let big = I2cReq { addr: 0x50, buf_ptr: anchor.as_ptr() as usize, len: 65 };
        check(
            "i2c read(len=65 > D-2)",
            ioctl(i2c, I2C_READ, &big as *const I2cReq as usize),
            EINVAL,
            &mut fails,
        );
        close(i2c);
    }
    let spi = open("/dev/spi0", O_RDONLY);
    if spi >= 0 {
        let spi = spi as i32;
        let bad = SpiReq { buf_ptr: BAD_PTR, len: 4 };
        check(
            "spi transfer(buf_ptr invalido)",
            ioctl(spi, SPI_TRANSFER, &bad as *const SpiReq as usize),
            EFAULT,
            &mut fails,
        );
        close(spi);
    }
    if fails == 0 {
        println!("[ioctltest] all tests passed");
        0
    } else {
        println!("[ioctltest] {} failure(s)", fails);
        1
    }
}
