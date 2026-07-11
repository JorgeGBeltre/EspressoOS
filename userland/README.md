# userland (futuro)

Aplicaciones de "usuario" que se ejecutan sobre el kernel usando exclusivamente
la API de syscalls (Fase 6+). Recordatorio de alcance: al no haber MMU de
paginación en el ESP32-S3, estos programas son tareas del kernel con contexto
propio (cwd, descriptores), no procesos con espacio de direcciones aislado.

Vacío por ahora — se poblará cuando la API de syscalls esté estable.
