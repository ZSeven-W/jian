/// Stable status values returned by every Jian C ABI call.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianStatus {
    Ok = 0,
    InvalidArg = 1,
    BadDocument = 2,
    LayoutError = 3,
    GpuError = 4,
    OutOfMemory = 5,
    WrongThread = 6,
    Suspended = 7,
    Busy = 8,
    NoFocus = 9,
    NotReady = 10,
    Poisoned = 11,
}
