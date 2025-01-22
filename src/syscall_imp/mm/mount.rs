use core::ffi::{c_char, c_void};
use arceos_posix_api as api;
pub(crate) fn sys_mount(
    source: *const c_char,
    target: *const c_char,
    fstype: *const c_char,
    flags: u64,
    data: *const c_void,
) -> i32 {
    api::sys_mount(source, target, fstype, flags, data)
}

pub(crate) fn sys_umount(target: *const c_char) -> i32 {
    api::sys_umount(target)
}
