/* Userland linker script — split Harvard para ejecutar desde PSRAM (Ruta B).
 *
 * El ESP32-S3 no puede hacer fetch de instrucciones desde el bus de datos
 * (0x3c...). El kernel mapea el 1 MB de PSRAM reservado en DOS buses:
 *   - Bus de INSTRUCCIONES en 0x42800000 (aquí va .text; el loader lo escribe
 *     por su alias de datos 0x3c0e0000 y luego se EJECUTA por 0x42800000).
 *   - Bus de DATOS en 0x3c0e0000..0x3c1e0000 (aquí va .data/.rodata/.bss).
 * .text y .data usan páginas físicas DISTINTAS (0-7 vs 8-15) para no solaparse.
 */
ENTRY(_start)

MEMORY
{
  ITEXT (rx) : ORIGIN = 0x42800000, LENGTH = 512K   /* PSRAM, bus de instrucciones (páginas 0-7) */
  UDATA (rw) : ORIGIN = 0x3c160000, LENGTH = 512K   /* PSRAM, bus de datos       (páginas 8-15) */
}

SECTIONS
{
  .text :
  {
    *(.literal._start)
    *(.text._start)
    *(.literal .literal.*)
    *(.text .text.*)
  } > ITEXT

  .rodata :
  {
    *(.rodata .rodata.*)
  } > UDATA

  .data :
  {
    *(.data .data.*)
  } > UDATA

  .bss :
  {
    *(.bss .bss.*)
  } > UDATA
}
