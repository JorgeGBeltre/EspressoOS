# build-userland.ps1 — Compila los programas de userland y los deja en userland/dist/.
#
# Cada app se enlaza a una direccion FIJA distinta dentro del 1 MB de PSRAM
# reservado para userland (0x3c000000..0x3c100000), en slots de 64 KB. Esto es
# necesario porque el backend Xtensa de LLVM NO soporta PIC/PIE, asi que los
# binarios son ET_EXEC estaticos: si dos procesos coexisten (p.ej. init + sh),
# necesitan direcciones distintas para no pisarse.
#
# Limitacion: un mismo binario no puede ejecutarse dos veces a la vez (comparten
# slot). Suficiente para init -> sh -> utilidad.
#
# Tras ejecutar esto, compila el kernel (cargo build/run): su build.rs empotra
# los ELF de dist/ y el kernel los instala en /bin al arrancar.
#
# Uso:  powershell -File tools/build-userland.ps1

$ErrorActionPreference = "Stop"
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH
. "$env:USERPROFILE\export-esp.ps1"

$root = Split-Path -Parent $PSScriptRoot          # EspressoOS/
$uland = Join-Path $root "userland"
$slots = Join-Path $uland ".slots"
$dist  = Join-Path $uland "dist"
New-Item -ItemType Directory -Force -Path $slots | Out-Null
New-Item -ItemType Directory -Force -Path $dist  | Out-Null

# app -> indice de slot (direccion = 0x3c000000 + i*0x10000)
$apps = [ordered]@{
  "init" = 0; "sh" = 1; "cat" = 2; "ls" = 3; "echo" = 4;
  "ota" = 5; "ping" = 6; "sntp" = 7; "netstat" = 8; "httpd" = 9
}

$tpl = @'
ENTRY(_start)
SECTIONS
{{
  . = 0x{0:x8};
  .text : {{ *(.literal._start) *(.text._start) *(.literal .literal.*) *(.text .text.*) }}
  .rodata : {{ *(.rodata .rodata.*) }}
  .data : {{ *(.data .data.*) }}
  .bss : {{ *(.bss .bss.*) }}
}}
'@

Set-Location $uland
$ok = 0
foreach ($name in $apps.Keys) {
  $addr = 0x3c000000 + ($apps[$name] * 0x10000)
  $script = "user_$name.x"
  ($tpl -f $addr) | Out-File -FilePath (Join-Path $slots $script) -Encoding ascii
  # RUSTFLAGS explicito => ANULA el rustflags heredado del .cargo/config ancestro
  # (que inyecta -Tlinkall.x del kernel y rompe el link del userland).
  $env:RUSTFLAGS = "-C link-arg=-nostartfiles -C force-frame-pointers -C link-arg=-L.slots -C link-arg=-T$script"
  Write-Output "== compilando $name @ 0x$("{0:x8}" -f $addr) =="
  cargo build --release --bin $name | Out-Null
  $out = Join-Path $uland "target\xtensa-esp32s3-none-elf\release\$name"
  if (Test-Path $out) {
    Copy-Item $out (Join-Path $dist "$name.elf") -Force
    $ok++
  } else {
    Write-Output "  !! fallo: no se genero $name"
  }
}
Write-Output "LISTO: $ok/$($apps.Count) binarios en $dist"
Get-ChildItem $dist -Filter *.elf | Select-Object Name,Length
