ENTRY(_start)

SECTIONS
{
  . = 0x3c000000;

  .text :
  {
    *(.literal._start)
    *(.text._start)
    *(.literal .literal.*)
    *(.text .text.*)
  }

  .rodata :
  {
    *(.rodata .rodata.*)
  }

  .data :
  {
    *(.data .data.*)
  }

  .bss :
  {
    *(.bss .bss.*)
  }
}
