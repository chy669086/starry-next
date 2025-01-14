use arceos_posix_api as api;

pub(crate) fn sys_clock_gettime(clock_id: i32, tp: *mut api::ctypes::timespec) -> i32 {
    unsafe { api::sys_clock_gettime(clock_id, tp) }
}

pub(crate) fn sys_get_time_of_day(tv: *mut api::ctypes::timeval) -> i32 {
    unsafe { api::sys_get_time_of_day(tv) }
}
