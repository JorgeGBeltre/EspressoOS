# Camino a Producción — EspressoOS

> Documento honesto de “qué falta para producción”. Estado hoy: **borrador sin
> compilar**. Este archivo NO declara el OS listo; enumera el trabajo real que
> falta, sus criterios de aceptación y quién puede hacerlo.

## 1. Qué significa “listo para producción” aquí (criterios de salida)

Un SO se considera de producción cuando TODO lo siguiente es cierto y verificado
en hardware real (ESP32-S3-WROOM-1-N16R8):

1. **Compila limpio** — `cargo build --release` sin errores ni warnings; `clippy`
   sin lints; `fmt` aplicado. Cero archivos en `COMPILE-STATUS: borrador`.
2. **Arranca de forma fiable** — 100/100 reinicios llegan a la shell sin colgarse.
3. **Multitarea correcta** — preempción segura (sin corrupción de pila/ventanas
   Xtensa) bajo carga; soak test de 72 h sin fugas de memoria ni deadlocks.
4. **Memoria** — SRAM + PSRAM en uso; sin fugas (heap estable en soak); protección
   W^X de regiones del kernel vía PMS.
5. **Almacenamiento persistente** — LittleFS en `/`; un archivo sobrevive a reboot
   y a corte de energía a mitad de escritura (prueba de power-loss).
6. **Red estable** — WiFi + TCP/IP sin panics ante tráfico malformado; reconexión
   automática; sin quedarse sin sockets.
7. **Seguridad** — ver `SECURITY.md`: cripto auditada, secretos protegidos, TRNG
   validado, entradas de red con límites, sin credenciales en el binario.
8. **Actualizable** — OTA A/B con rollback probado (imagen mala → vuelve a la buena).
9. **Observabilidad** — panics con backtrace + coredump a partición; logs con nivel.
10. **CI verde** — build + clippy + fmt + tests de lógica en cada commit.
11. **Reproducible** — versiones fijadas; build reproducible documentado.

## 2. El bottleneck (léelo)

El único camino a producción pasa por el ciclo **compilar → flashear → depurar en
hardware**. Eso requiere, en la máquina del desarrollador:
- `espup install` (toolchain Xtensa) + `espflash`.
- La placa ESP32-S3 conectada por USB.

Sin eso, ningún avance de código es *verificable*. La acción de mayor impacto para
producción **no es escribir más código**: es instalar el toolchain y entrar al
ciclo. A partir de ahí se convierte “borrador” en “compila” archivo a archivo.

## 3. Leyenda de propiedad

- 🖥️ **Requiere toolchain/HW** (solo el desarrollador puede cerrarlo).
- 🧩 **Lógica pura** (verificable en host; se puede avanzar sin HW).
- 📄 **Infra/documento** (no necesita HW).

## 4. Plan por hitos (P0 → P9)

| Hito | Objetivo | Gate de aceptación | Owner |
|---|---|---|---|
| **P0** Compila | `cargo build --release` enlaza | 0 errores; arreglar drift API esp-hal 0.23 en los `(?)` | 🖥️ |
| **P1** Arranca | bring-up + shell sobre ramfs | banner + LED + shell interactiva en placa | 🖥️ |
| **P2** Contexto | validar switch Xtensa | 3 tareas **cooperativas** (yield) estables | 🖥️ |
| **P3** Preempción | `need_resched` en epílogo de vector | tarea que no cede es expulsada; sin corrupción | 🖥️ |
| **P4** Memoria | cablear PSRAM + PMS W^X | `free` = SRAM+PSRAM; escritura a región del kernel falla | 🖥️ |
| **P5** Persistencia | LittleFS real en `/` | archivo sobrevive a reboot y a power-loss | 🖥️ |
| **P6** Red | `wifi::init_hw` + **listen TCP** | servidor TCP responde; sin panic ante fuzz básico | 🖥️ |
| **P7** SSH | máquina de estados sobre `proto` (ya probado) | `ssh user@esp32` da shell contra `ssh -vvv` | 🖥️🧩 |
| **P8** Endurecer | límites, timeouts, rekey, zeroize, coredump | soak 72 h; suite de fuzz de red; sin fugas | 🖥️ |
| **P9** Release | versionado, CI verde, OTA rollback | checklist §6 completo; tag firmado | 🖥️📄 |

Notas de arquitectura que P2–P6 deben resolver (del análisis del kernel):
- **Preempción desde ISR (P3):** hoy `tick → switch_to` corre DENTRO del `#[handler]`;
  cambiar a marcar `need_resched` y conmutar en el epílogo del vector.
- **Secciones críticas (P3/P6):** el `Mutex` enmascara TODAS las interrupciones;
  `flash`/`wifi` no deben mantenerlo durante operaciones largas (mata la preempción
  y priva de IRQs al firmware WiFi).
- **Frontera de syscalls (P7):** `syscall::dispatch` está escrito pero es código
  inalcanzable (sin vector); decidir si se cablea o si el aislamiento se pospone.

## 5. Qué se puede avanzar SIN hardware (🧩/📄)

- Ampliar la cobertura de **lógica pura probada** (hoy: shell, VFS, OTA, ramfs, SSH
  proto = 93 tests verdes). Candidatos: más casos de rutas VFS, secuencia/rollback
  OTA, framing de canales SSH, generador de particiones vs formato esp-idf.
- **Diseño** de las piezas de HW antes de tocarlas (reduce iteraciones en placa).
- **Infra**: CI (`.github/workflows/ci.yml`), `SECURITY.md`, `.gitattributes`,
  runner unificado de tests.

## 6. Checklist de release (P9)

- [ ] `cargo build --release` limpio · `clippy` sin lints · `fmt` aplicado
- [ ] 0 archivos `COMPILE-STATUS: borrador`
- [ ] 100/100 arranques a shell
- [ ] Soak 72 h sin fugas ni deadlocks
- [ ] Power-loss test del FS
- [ ] Suite de fuzz de red sin panics
- [ ] OTA A/B con rollback probado
- [ ] `SECURITY.md` completo y revisado
- [ ] CI verde en `main`
- [ ] Versión etiquetada + notas de build reproducible

## 7. Estado real hoy (para no engañarnos)

- ~7 000 LOC de kernel: **borrador, sin compilar**.
- Verificado de verdad: **solo lógica pura** (93 tests Python).
- P0 aún no empezado (sin toolchain). Todo P1–P9 pendiente.
- **Conclusión:** el OS NO está listo para producción y no puede estarlo hasta
  ejecutar el ciclo de HW. Este documento es el mapa para llegar.
