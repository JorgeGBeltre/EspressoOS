# Seguridad — EspressoOS

Modelo de amenazas y requisitos de endurecimiento para producción. Aplica sobre
todo a las superficies expuestas: **red (WiFi/TCP), SSH y OTA**.

## Principios

1. **Cero criptografía propia.** Toda primitiva (cifrado, hash, firma, KDF, ECDH)
   viene de crates auditadas `no_std` (RustCrypto / dalek). El kernel solo orquesta.
   Ver `docs/ssh-design.md` §5.
2. **Sin secretos en el binario.** Claves de host, contraseñas y claves autorizadas
   viven en el FS (LittleFS), nunca hardcodeadas ni en logs.
3. **Entradas de red = hostiles.** Todo parser (SSH `proto`, DHCP, TCP) valida
   longitudes y límites antes de asignar; nunca hace panic ante datos malformados.
4. **Tiempo constante** para comparar secretos (contraseñas, etiquetas MAC): usar
   `subtle::ConstantTimeEq`, nunca `==`.
5. **Menor privilegio de memoria (W^X).** Programar el PMS/World Controller para que
   las regiones de código del kernel no sean escribibles y las de datos no
   ejecutables (Fase 8).

## Checklist de endurecimiento (gate de P7/P8)

### Criptografía / SSH
- [ ] Clave de host `ssh-ed25519` generada con el **TRNG por HW** y guardada en FS.
- [ ] **TRNG validado**: el RNG del ESP32-S3 solo es TRNG real con la radio/ADC
      activa; verificar antes de usarlo para claves efímeras. Documentar la fuente.
- [ ] Secreto compartido X25519 y claves de sesión con `zeroize` al cerrar sesión.
- [ ] Verificación de firma y de MAC en **tiempo constante**.
- [ ] Rechazo de algoritmos débiles (nada de RSA-SHA1, AES-CBC, HMAC-SHA1, DH-group1).
- [ ] `rekey` por volumen/tiempo (RFC 4253 §9): re-negociar antes de 1 GiB / 1 h.

### Autenticación
- [ ] Contraseñas comparadas contra un **hash** (no texto plano), en tiempo constante.
- [ ] Límite de intentos + backoff; desconexión tras N fallos.
- [ ] `publickey` contra lista de claves autorizadas por usuario (authorized_keys).
- [ ] Sin usuarios/contraseñas por defecto en el binario de release.

### Superficie de red
- [ ] Límites de tamaño en TODO parser (SSH packet ≤ 35 000; strings acotados).
- [ ] Timeouts de handshake y de sesión inactiva.
- [ ] Límite de conexiones concurrentes (evitar agotar sockets/heap).
- [ ] Fuzzing básico del parser SSH y del stack (entradas aleatorias sin panic).

### Memoria / robustez
- [ ] W^X vía PMS para regiones del kernel.
- [ ] Sin `static mut` fuera de las excepciones documentadas.
- [ ] `panic` del kernel → coredump a partición + reinicio limpio (no cuelgue).
- [ ] Overflow checks revisados en rutas que tocan longitudes de red.

### OTA
- [ ] Verificación de **firma** de la imagen antes de conmutar de slot.
- [ ] Rollback automático si la imagen nueva no se auto-valida tras N arranques.
- [ ] `otadata` con doble copia + CRC (ya implementado en `ota/partition.rs`).

## Reporte de vulnerabilidades

Proyecto personal/educativo. Para uso real, definir un contacto de seguridad y un
proceso de divulgación responsable antes del primer despliegue expuesto a internet.

> **Aviso:** un servidor SSH escrito desde cero NO debe exponerse a internet hasta
> pasar revisión de seguridad independiente e interoperar de forma estable con
> clientes reales. En LAN de desarrollo, aún así, aplica el checklist de arriba.
