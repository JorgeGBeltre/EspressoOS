# Diseño — Servidor SSH para EspressoOS

> Estado: **borrador de diseño + esqueleto**. Nada compila ni corre aún (ver
> "Prerrequisitos"). La criptografía se **delega a crates auditadas `no_std`**;
> este kernel NO implementa primitivas criptográficas propias.

## 1. Alcance

Un **servidor SSH-2.0 mínimo** (RFC 4251/4253/4252/4254) que permita abrir una
sesión de shell remota sobre TCP (puerto 22), reutilizando la shell existente
(`shell::run`) a través de una abstracción de E/S (`shell::remote`).

MVP del servidor:
- Intercambio de versión + **KEXINIT**.
- **Key exchange**: `curve25519-sha256` (RFC 8731).
- **Host key / firma de servidor**: `ssh-ed25519` (RFC 8709).
- **Cifrado AEAD**: `chacha20-poly1305@openssh.com` (sin MAC separada).
- **Autenticación de usuario**: `password` y `publickey` (ssh-ed25519).
- **Canal** `session` + `pty-req` + `shell` → conecta con `shell::remote`.
- Sin compresión, sin reenvío de puertos, sin agente.

Fuera de alcance (por ahora): múltiples canales simultáneos, `exec`/`subsystem`
(SFTP), rekey por volumen, algoritmos legacy (RSA, AES-CBC, HMAC-SHA1).

## 2. Prerrequisitos (gating honesto)

SSH corre sobre TCP; **no puede funcionar** hasta que:
1. El kernel **compile** (hoy 36 archivos en `borrador`, sin toolchain).
2. **Fase 7 (red) esté cableada**: `drivers::wifi::init_hw` invocado desde `main`,
   y el stack `smoltcp` procesando paquetes. Además hoy `wifi.rs` sólo expone
   cliente TCP; falta un **socket TCP en modo escucha** (listen) en puerto 22.
3. El `Mutex` canónico no monopolice las interrupciones durante el `poll` de red
   (ver brecha de secciones críticas del análisis del kernel).

Por eso, lo que se entrega ahora es **diseño + esqueleto + wire-format probado**,
no un servidor ejecutable. Se marca todo con `// COMPILE-STATUS: borrador`.

## 3. Ubicación en el árbol

Siguiendo la estructura propuesta (`kernel/src/drivers/ssh/`). Nota de altitud:
SSH es un **servicio de aplicación**, no un driver de dispositivo; una ubicación
más limpia sería `kernel/src/net/ssh/` o `services/ssh/`. Se mantiene bajo
`drivers/` por continuidad con la estructura acordada; mover es trivial más tarde.

```
kernel/src/drivers/ssh/
├── mod.rs      # servidor: listener + máquina de estados de la conexión
├── proto.rs    # binary packet protocol + tipos RFC 4251 (LÓGICA PURA, con tests)
├── kex.rs      # curve25519-sha256: hash de intercambio + derivación de claves
├── auth.rs     # userauth: password + publickey (ssh-ed25519)
├── channel.rs  # capa de canales: session/pty-req/shell -> shell::remote
└── crypt.rs    # capa AEAD: chacha20-poly1305@openssh.com (via RustCrypto)
kernel/src/shell/remote.rs   # abstracción de E/S: shell sobre un stream (consola o canal SSH)
```

## 4. Máquina de estados de la conexión

```
TCP accept
   │
   ▼
[VersionExchange]  intercambio de "SSH-2.0-EspressoOS_0.1\r\n"
   │
   ▼
[KexInit]          enviar/recibir SSH_MSG_KEXINIT (negociar algoritmos)
   │
   ▼
[Kex]              SSH_MSG_KEX_ECDH_INIT/REPLY (curve25519), calcular H y K,
   │               derivar claves; SSH_MSG_NEWKEYS en ambos sentidos
   ▼
[Encrypted]        a partir de aquí todo va cifrado (crypt::Aead)
   │
   ▼
[ServiceRequest]   "ssh-userauth"
   │
   ▼
[UserAuth]         password / publickey  → SSH_MSG_USERAUTH_SUCCESS
   │
   ▼
[Connection]       CHANNEL_OPEN(session) + CHANNEL_REQUEST(pty-req, shell)
   │
   ▼
[Session]          bucle: CHANNEL_DATA <-> shell::remote (E/S de la shell)
```

## 5. Criptografía (crates auditadas, `no_std`)

| Función | Algoritmo | Crate (RustCrypto) |
|---|---|---|
| Key exchange | X25519 (ECDH) | `x25519-dalek` v2 |
| Hash de kex/KDF | SHA-256 | `sha2` |
| Firma host key | Ed25519 | `ed25519-dalek` v2 |
| Cifrado de sesión | ChaCha20-Poly1305 (constr. openssh) | `chacha20` + `poly1305` |
| Comparaciones | tiempo constante | `subtle` |
| Borrado de secretos | zeroize | `zeroize` |
| Aleatoriedad | TRNG del ESP32-S3 | `esp_hal::rng::Rng` → `rand_core` |

**Regla dura:** ninguna primitiva criptográfica se implementa a mano en este
kernel. `crypt.rs`/`kex.rs` sólo *orquestan* llamadas a estas crates. La única
"lógica de bytes" propia es el framing y los codecs de `proto.rs`, que NO son
criptografía y por eso sí se prueban con un arnés en Python.

**Fuente de entropía:** el servidor necesita un RNG criptográfico para la clave
efímera X25519 y el padding. Se usa el **TRNG por hardware** del ESP32-S3
(`esp_hal::rng::Rng`), envuelto en un adaptador `rand_core::CryptoRng`. (Nota de
seguridad: el TRNG de esp-hal requiere la radio/ADC activa para ser un TRNG real;
verificar en HW.)

> **Aviso de versiones:** las versiones de las crates son la mejor estimación
> `no_std` compatible con esp-hal 0.23; confirmar al compilar (igual que el resto
> del kernel). La construcción `chacha20-poly1305@openssh.com` NO es la del crate
> `chacha20poly1305` (que es RFC 8439): usa dos instancias de ChaCha20 (una para
> la longitud, otra para el payload) y Poly1305 con clave derivada del keystream;
> por eso se compone a mano en `crypt.rs` a partir de `chacha20` + `poly1305`.

## 6. Puente con la shell (`shell::remote`)

Hoy `shell::run` está atada a la consola (`drivers::uart::getc`/`write`). Para el
shell remoto se introduce un trait de E/S:

```rust
pub trait ShellIo {
    fn read_byte(&mut self) -> Option<u8>;   // None = sin datos ahora (cede CPU)
    fn write(&mut self, bytes: &[u8]) -> usize;
}
```

- `ConsoleIo` (adaptador sobre `drivers::uart`) para la sesión local.
- `SshChannelIo` (adaptador sobre un canal SSH) para la sesión remota.

El bucle REPL pasa a `run_with_io(io: &mut dyn ShellIo)`; `run()` local llama a
`run_with_io(&mut ConsoleIo)`. Así la MISMA shell sirve local y remota. (Este
refactor es parte del esqueleto; el `shell::run` actual se conserva como wrapper.)

## 7. Verificación

- **`proto.rs`** (framing + tipos RFC 4251): probado con `tools/tests/ssh_proto_tests.py`
  (round-trip de byte/bool/uint32/string/mpint/name-list y del binary packet
  protocol: longitudes, padding mínimo 4, múltiplo de bloque). Es la única capa
  verificable sin compilador/hardware/cripto.
- **kex/auth/channel/crypt**: sólo verificables en HW con el toolchain, contra un
  cliente `ssh` real (`ssh -vvv`). Se valida por interoperabilidad, no por tests
  unitarios de cripto (esos ya los cubren las crates auditadas).

## 8. Orden de trabajo recomendado

1. (Prerrequisito) Compilar el kernel + cablear red (Fase 7) + añadir socket TCP
   *listen* en `drivers::wifi`.
2. `proto.rs` a firme (ya probado en wire-format) → añadir tests en dispositivo.
3. Version exchange + KEXINIT + kex X25519 + NEWKEYS contra `ssh -vvv` (sin auth).
4. `crypt.rs` (AEAD) → primer paquete cifrado correcto.
5. `auth.rs` (password primero, luego publickey).
6. `channel.rs` + `shell::remote` → shell remota funcional.
7. Endurecer: límites de tamaño, rekey, timeouts, borrado de secretos (zeroize).
```
