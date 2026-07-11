# mkimage (ESQUELETO)

Empaquetador de la imagen del kernel con la cabecera de arranque del ESP32-S3.

> **Nota:** en la Fase 0 esto **no es necesario**: `espflash` ya genera la
> imagen en el formato correcto de Espressif (magic `0xE9`, segmentos, hash) al
> flashear. El plan lo listaba como "header Multiboot 2", pero Multiboot2 es de
> GRUB/PC y no aplica al ESP32; el formato real es el de es-idf.

Esta herramienta solo cobra sentido cuando exista el **bootloader propio de 2ª
etapa** (fase avanzada) y queramos empaquetar el kernel sin depender de
espflash. Hasta entonces queda como marcador de posición.

Pendiente:
- Leer el ELF del kernel, extraer segmentos y sus direcciones de carga.
- Emitir `EspImageHeader` + segmentos + hash SHA-256.
