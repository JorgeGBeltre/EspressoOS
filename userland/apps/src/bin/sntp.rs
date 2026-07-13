#![no_std]
#![no_main]

use libc::{println, socket, connect, write, read, close, timeval, settimeofday, sockaddr_in, yield_now};

#[no_mangle]
pub extern "C" fn main() -> i32 {
    println!("[sntp] Iniciando sincronizacion de hora...");

    let fd = socket(2, 2, 0); // AF_INET = 2, SOCK_DGRAM = 2
    if fd < 0 {
        println!("[sntp] Error al crear socket UDP");
        return -1;
    }

    let ip: u32 = u32::from_ne_bytes([128, 138, 140, 44]); // 128.138.140.44
    let port = 123u16;

    let addr = sockaddr_in {
        sin_family: 2,
        sin_port: port.to_be(),
        sin_addr: ip,
        sin_zero: [0; 8],
    };

    println!("[sntp] Conectando a 128.138.140.44:123...");
    if connect(fd as i32, &addr) < 0 {
        println!("[sntp] Error al conectar el socket UDP");
        close(fd as i32);
        return -1;
    }

    let mut pkt = [0u8; 48];
    pkt[0] = 0x1B; // LI=0, VN=3, Mode=3 (Client)

    println!("[sntp] Enviando peticion SNTP...");
    if write(fd as i32, &pkt) < 0 {
        println!("[sntp] Error al enviar peticion");
        close(fd as i32);
        return -1;
    }

    println!("[sntp] Esperando respuesta...");
    let mut resp = [0u8; 48];
    let mut attempts = 0;
    loop {
        let n = read(fd as i32, &mut resp);
        if n > 0 {
            if n == 48 {
                break;
            }
        }
        attempts += 1;
        if attempts > 200 {
            println!("[sntp] Timeout al recibir respuesta");
            close(fd as i32);
            return -1;
        }
        yield_now();
    }

    let seconds_1900 = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]);
    
    let ntp_offset = 2208988800u32;
    if seconds_1900 < ntp_offset {
        println!("[sntp] Error: timestamp recibido es invalido");
        close(fd as i32);
        return -1;
    }
    let unix_secs = seconds_1900 - ntp_offset;
    
    let tv = timeval {
        tv_sec: unix_secs as i32,
        tv_usec: 0,
    };

    println!("[sntp] Hora recibida: {} s (UNIX Epoch). Seteando reloj...", unix_secs);
    if settimeofday(&tv) < 0 {
        println!("[sntp] Error al actualizar settimeofday");
        close(fd as i32);
        return -1;
    }

    println!("[sntp] Reloj sincronizado con exito!");
    close(fd as i32);
    0
}
