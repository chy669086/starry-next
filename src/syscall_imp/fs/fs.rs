use arceos_posix_api as api;
use arceos_posix_api::ctypes::{mode_t, size_t};
use core::ffi::c_char;

pub(crate) fn sys_openat(dirfd: i32, path: *const c_char, flags: i32, modes: mode_t) -> isize {
    api::sys_openat(dirfd, path, flags, modes) as isize
}

pub(crate) fn sys_close(fd: i32) -> i32 {
    api::sys_close(fd)
}

pub(crate) fn sys_dup(fd: i32) -> i32 {
    api::sys_dup(fd)
}

pub(crate) fn sys_dup2(old_fd: i32, new_fd: i32) -> i32 {
    api::sys_dup2(old_fd, new_fd)
}

pub(crate) fn sys_dup3(old_fd: i32, new_fd: i32, flags: i32) -> i32 {
    // api::sys_dup3(old_fd, new_fd, flags)
    sys_dup2(old_fd, new_fd)
}

pub(crate) fn sys_getcwd(buf: *mut c_char, size: size_t) -> *mut c_char {
    api::sys_getcwd(buf, size)
}

pub(crate) fn sys_chdir(filename: *const c_char) -> i32 {
    api::sys_chdir(filename)
}

pub(crate) fn sys_mkdirat(dirfd: i32, pathname: *const c_char, mode: mode_t) -> i32 {
    api::sys_mkdirat(dirfd, pathname, mode)
}
