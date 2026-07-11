//! Primitivas de sincronización de bajo nivel del kernel (Xtensa LX7).
//!
//! Contiene tres primitivas, de la más cruda a la más ergonómica:
//! - [`SpinLock`]: cerrojo atómico crudo (sin dormir, sin guard). Es la base
//!   sobre la que se construye todo lo demás. NO enmascara interrupciones.
//! - [`Mutex`] / [`MutexGuard`]: cerrojo con guard RAII que protege un dato `T`.
//!   Mientras se mantiene tomado, enmascara las interrupciones del núcleo para
//!   evitar el auto-bloqueo que ocurriría si una ISR intentara adquirir el
//!   mismo cerrojo que la tarea interrumpida (patrón `spin_lock_irqsave` de los
//!   kernels tipo Linux). Es la primitiva recomendada para el estado global.
//! - [`CriticalSection`]: guard RAII que ejecuta una sección crítica con las
//!   interrupciones deshabilitadas, apoyándose en
//!   [`super::interrupts::disable`] / [`super::interrupts::restore`].
//!
//! Regla del contrato (§0.7 / §6.7): el estado global mutable se protege con
//! `SpinLock`, con `Mutex<T>` o dentro de una sección crítica; jamás con
//! `static mut` desnudo (salvo el buffer del heap).
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use super::interrupts;

// =============================================================================
// SpinLock: primitiva base (atómica, sin guard, sin enmascarar interrupciones).
// =============================================================================

/// Spinlock simple (sin dormir). Suficiente para secciones muy cortas.
///
/// Firma CANÓNICA del contrato (§3.2.4): `new`, `lock`, `unlock` NO se tocan.
/// Es una primitiva cruda: por sí sola NO deshabilita interrupciones, así que
/// un `SpinLock` compartido entre una tarea y una ISR del mismo núcleo puede
/// auto-bloquearse. Para ese caso usar [`Mutex`] o [`CriticalSection`].
pub struct SpinLock {
    locked: AtomicBool,
}

impl SpinLock {
    /// Crea un spinlock libre. `const` para poder usarlo en `static`.
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    /// Adquiere el cerrojo, girando (busy-wait) hasta conseguirlo.
    pub fn lock(&self) {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Sugerencia al CPU de que estamos en un bucle de espera activa.
            core::hint::spin_loop();
        }
    }

    /// Libera el cerrojo. Debe llamarlo el mismo flujo que hizo `lock`.
    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }

    /// Intento no bloqueante: devuelve `true` si se adquirió el cerrojo.
    ///
    /// Adición al contrato: es una firma NUEVA (no cambia las canónicas) y la
    /// usa internamente [`Mutex::try_lock`].
    pub fn try_lock(&self) -> bool {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    /// Consulta si está tomado (diagnóstico; sin garantías de sincronización).
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

impl Default for SpinLock {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// CriticalSection: guard RAII de "interrupciones deshabilitadas".
// =============================================================================

/// Sección crítica con guard RAII. Al construirse deshabilita las
/// interrupciones del núcleo y al soltarse (drop) restaura el estado previo.
///
/// Complementa a `interrupts::critical_section(f)` (variante por closure del
/// contrato §3.2.2): esta versión con guard es cómoda cuando la sección abarca
/// varias sentencias y prefieres el alcance léxico de una variable.
///
/// # Ejemplo
/// ```ignore
/// {
///     let _cs = CriticalSection::enter(); // interrupciones OFF
///     // ... acceso exclusivo a estado compartido con ISRs ...
/// } // aquí se restauran las interrupciones automáticamente
/// ```
///
/// # Cuidado
/// No mantengas la sección abierta durante operaciones largas: mientras viva
/// este guard, el núcleo no atiende ninguna interrupción (afecta a la latencia
/// y al tick del scheduler). No la mantengas viva a través de un `yield`.
pub struct CriticalSection {
    /// Estado de interrupciones (PS) devuelto por `interrupts::disable`.
    prev_state: u32,
    /// Marca el guard como `!Send`/`!Sync`: el estado de interrupciones es por
    /// núcleo, así que el guard no debe cruzar de hilo/núcleo.
    _not_send: PhantomData<*const ()>,
}

impl CriticalSection {
    /// Entra en la sección crítica: enmascara interrupciones y captura el
    /// estado previo para restaurarlo al hacer drop.
    #[inline]
    pub fn enter() -> Self {
        let prev_state = interrupts::disable();
        Self {
            prev_state,
            _not_send: PhantomData,
        }
    }
}

impl Drop for CriticalSection {
    #[inline]
    fn drop(&mut self) {
        // Restaura EXACTAMENTE el estado previo (no habilita a ciegas): permite
        // anidar secciones críticas de forma segura.
        interrupts::restore(self.prev_state);
    }
}

// =============================================================================
// Mutex<T> / MutexGuard: cerrojo con guard que protege un dato, IRQ-safe.
// =============================================================================

/// Cerrojo con guard RAII que protege un dato `T`. Envuelve un [`SpinLock`]
/// (protección entre flujos / previsión SMP) y, además, enmascara las
/// interrupciones mientras se mantiene tomado (previsión ISR: evita el
/// auto-bloqueo tarea<->ISR en un mismo núcleo).
///
/// Firma CANÓNICA del contrato (§3.2.4): `new` (const) y `lock() -> MutexGuard`.
/// El enmascarado de interrupciones es un detalle INTERNO: no cambia la API
/// pública, solo hace el cerrojo seguro frente a ISRs por defecto.
///
/// Uso típico para estado global del kernel:
/// ```ignore
/// static TABLA: Mutex<Option<Tabla>> = Mutex::new(None);
/// // ...
/// let mut g = TABLA.lock();
/// *g = Some(Tabla::new());
/// ```
pub struct Mutex<T: ?Sized> {
    /// Cerrojo de exclusión (spin). Se toma DESPUÉS de enmascarar IRQs.
    lock: SpinLock,
    /// Dato protegido. El acceso mutable solo ocurre a través del guard.
    data: UnsafeCell<T>,
}

// El dato se comparte de forma segura entre flujos porque el acceso está
// serializado por el cerrojo. Requiere `T: Send` (el dato "viaja" al flujo que
// toma el cerrojo). Igual criterio que `spin::Mutex` / `std::sync::Mutex`.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Crea un mutex que envuelve `value`. `const` para poder usarlo en
    /// `static` sin inicialización perezosa.
    pub const fn new(value: T) -> Self {
        Self {
            lock: SpinLock::new(),
            data: UnsafeCell::new(value),
        }
    }

    /// Consume el mutex y devuelve el dato interno (sin cerrojo, hay acceso
    /// exclusivo garantizado por `self` por valor).
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Bloquea y devuelve un guard con `Deref`/`DerefMut`. Al soltarlo se libera
    /// el cerrojo y se restauran las interrupciones.
    ///
    /// Orden CRÍTICO (patrón irqsave): 1) deshabilitar IRQs, 2) tomar el spin.
    /// Al liberar (en `Drop`) el orden es inverso: 1) soltar spin, 2) restaurar
    /// IRQs. Así una ISR nunca puede interrumpirnos con el spin ya tomado.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        let irq_state = interrupts::disable();
        self.lock.lock();
        MutexGuard {
            mutex: self,
            irq_state,
            _not_send: PhantomData,
        }
    }

    /// Intento no bloqueante. `Some(guard)` si se adquirió; `None` si estaba
    /// ocupado (en cuyo caso se restauran de inmediato las interrupciones).
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        let irq_state = interrupts::disable();
        if self.lock.try_lock() {
            Some(MutexGuard {
                mutex: self,
                irq_state,
                _not_send: PhantomData,
            })
        } else {
            // No conseguimos el cerrojo: deshacer el enmascarado antes de salir.
            interrupts::restore(irq_state);
            None
        }
    }

    /// Acceso mutable directo cuando se tiene `&mut self` (hay exclusión
    /// estática garantizada por el borrow checker; no toca el cerrojo).
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: `&mut self` implica acceso exclusivo, no hay concurrencia.
        unsafe { &mut *self.data.get() }
    }
}

/// Guard de [`Mutex`]. Mientras vive, mantiene el cerrojo tomado y las
/// interrupciones enmascaradas. Da acceso al dato vía `Deref`/`DerefMut`.
///
/// Es `!Send`/`!Sync` a propósito (`PhantomData<*const ()>`): el estado de
/// interrupciones capturado es por núcleo, por lo que el guard no debe moverse
/// a otro flujo/núcleo ni compartirse.
pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,
    /// Estado de interrupciones a restaurar al soltar el guard.
    irq_state: u32,
    _not_send: PhantomData<*const ()>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: mientras el guard viva, el cerrojo está tomado, así que somos
        // el único acceso vivo al dato.
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: igual que en `deref`; además `&mut self` impide aliasing del
        // propio guard.
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        // Orden inverso al de `lock`: primero soltar el cerrojo, luego restaurar
        // las interrupciones. Nunca al revés (evita ventana de reentrada).
        self.mutex.lock.unlock();
        interrupts::restore(self.irq_state);
    }
}
