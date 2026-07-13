#![no_std]
#![no_main]

use libc::{println, socket, connect, close, uptime_ms, sockaddr_in};

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut ip = [0u8; 4];
    let mut part = 0;
    let mut val = 0u32;
    let mut has_digits = false;
    for &b in s.as_bytes() {
        if b == b'.' {
            if part >= 3 || !has_digits {
                return None;
            }
            ip[part] = val as u8;
            part += 1;
            val = 0;
            has_digits = false;
        } else if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u32;
            if val > 255 {
                return None;
            }
            has_digits = true;
        } else {
            return None;
        }
    }
    if part != 3 || !has_digits {
        return None;
    }
    ip[3] = val as u8;
    Some(ip)
}

#[no_mangle]
pub fn main() -> i32 {
    // TCP ping a un IP y puerto (puerto 80 por defecto)
    // El primer argumento no lo podemos obtener fácilmente en esta libc simple
    // a menos que parseemos argumentos o usemos un IP por defecto.
    // Vamos a usar una IP por defecto o una IP de prueba, o intentar obtenerla.
    // Como el shell ejecuta comandos pasando argumentos, vamos a ver cómo se obtienen
    // argumentos en EspressoOS userland!
    // Espera, ¿tiene EspressoOS soporte para argumentos de entrada en main?
    // En userland/libc/src/lib.rs, `_start` llama a `main()` sin argumentos!
    // Sí, `fn main() -> i32;` no tiene argumentos.
    // Así que usaremos una IP por defecto (e.g. 192.168.1.1 o 8.8.8.8) o consultaremos
    // el AP Gateway si está disponible, o simplemente haremos un TCP ping a 192.168.1.1
    // que es la IP estándar de los routers AP.
    let ip_str = "192.168.1.1";
    let ip = match parse_ip(ip_str) {
        Some(addr) => addr,
        None => {
            println!("ping: IP invalida");
            return 1;
        }
    };
    
    println!("PING {} puerto 80 (TCP)...", ip_str);
    
    let fd = socket(2, 1, 0); // AF_INET, SOCK_STREAM
    if fd < 0 {
        println!("ping: no se pudo crear el socket");
        return 1;
    }
    
    // Convertir IP a entero de 32 bits
    let ip_u32 = ((ip[0] as u32) << 0)
        | ((ip[1] as u32) << 8)
        | ((ip[2] as u32) << 16)
        | ((ip[3] as u32) << 24);
        
    let addr = sockaddr_in {
        sin_family: 2, // AF_INET
        sin_port: 80u16.to_be(),
        sin_addr: ip_u32,
        sin_zero: [0; 8],
    };
    
    let start = uptime_ms();
    let ret = connect(fd as i32, &addr);
    let end = uptime_ms();
    
    if ret == 0 {
        println!("Conexion exitosa con {}, RTT = {} ms", ip_str, end - start);
    } else {
        println!("Fallo al conectar con {} (codigo {})", ip_str, ret);
    }
    
    close(fd as i32);
    0
}
