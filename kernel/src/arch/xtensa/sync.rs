#![allow(dead_code)]

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use super::interrupts;

pub struct SpinLock {
    locked: AtomicBool,
}

impl SpinLock {

    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    pub fn lock(&self) {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {

            core::hint::spin_loop();
        }
    }

    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }

    pub fn try_lock(&self) -> bool {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

impl Default for SpinLock {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CriticalSection {

    prev_state: u32,

    _not_send: PhantomData<*const ()>,
}

impl CriticalSection {

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

        interrupts::restore(self.prev_state);
    }
}

pub struct Mutex<T: ?Sized> {

    lock: SpinLock,

    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {

    pub const fn new(value: T) -> Self {
        Self {
            lock: SpinLock::new(),
            data: UnsafeCell::new(value),
        }
    }

    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized> Mutex<T> {

    pub fn lock(&self) -> MutexGuard<'_, T> {
        let irq_state = interrupts::disable();
        self.lock.lock();
        MutexGuard {
            mutex: self,
            irq_state,
            _not_send: PhantomData,
        }
    }



    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        let irq_state = interrupts::disable();
        if self.lock.try_lock() {
            Some(MutexGuard {
                mutex: self,
                irq_state,
                _not_send: PhantomData,
            })
        } else {

            interrupts::restore(irq_state);
            None
        }
    }

    pub fn get_mut(&mut self) -> &mut T {

        unsafe { &mut *self.data.get() }
    }
}

pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,

    irq_state: u32,
    _not_send: PhantomData<*const ()>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {


        unsafe { &*self.mutex.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {

        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {

        self.mutex.lock.unlock();
        interrupts::restore(self.irq_state);
    }
}
