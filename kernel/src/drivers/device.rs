use crate::prelude::*;
use crate::arch::xtensa::Mutex;
use alloc::collections::BTreeMap;

pub trait Device: Send + Sync {
    fn read(&self, offset: usize, buf: &mut [u8]) -> KResult<usize>;
    fn write(&self, offset: usize, buf: &[u8]) -> KResult<usize>;
    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<i32> {
        Err(KError::NotSupported)
    }
}

static DEVICES: Mutex<Option<BTreeMap<String, Arc<dyn Device>>>> = Mutex::new(None);

pub fn init() {
    crate::arch::xtensa::interrupts::critical_section(|| {
        let mut map = BTreeMap::new();
        map.insert(String::from("null"), Arc::new(NullDevice) as Arc<dyn Device>);
        map.insert(String::from("zero"), Arc::new(ZeroDevice) as Arc<dyn Device>);
        map.insert(String::from("console"), Arc::new(ConsoleDevice) as Arc<dyn Device>);
        *DEVICES.lock() = Some(map);
    });
}

pub fn register_device(name: &str, device: Arc<dyn Device>) {
    crate::arch::xtensa::interrupts::critical_section(|| {
        let mut guard = DEVICES.lock();
        if let Some(map) = guard.as_mut() {
            map.insert(String::from(name), device);
        }
    });
}

pub fn get_device(name: &str) -> Option<Arc<dyn Device>> {
    crate::arch::xtensa::interrupts::critical_section(|| {
        let guard = DEVICES.lock();
        guard.as_ref().and_then(|map| map.get(name).cloned())
    })
}

pub fn list_devices() -> Vec<String> {
    crate::arch::xtensa::interrupts::critical_section(|| {
        let guard = DEVICES.lock();
        guard.as_ref()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_else(Vec::new)
    })
}

struct NullDevice;
impl Device for NullDevice {
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> KResult<usize> {
        Ok(0)
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> KResult<usize> {
        Ok(buf.len())
    }
}

struct ZeroDevice;
impl Device for ZeroDevice {
    fn read(&self, _offset: usize, buf: &mut [u8]) -> KResult<usize> {
        buf.fill(0);
        Ok(buf.len())
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> KResult<usize> {
        Ok(buf.len())
    }
}

struct ConsoleDevice;
impl Device for ConsoleDevice {
    fn read(&self, _offset: usize, buf: &mut [u8]) -> KResult<usize> {
        Ok(crate::drivers::uart::read(buf))
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> KResult<usize> {
        Ok(crate::drivers::uart::write(buf))
    }
}
