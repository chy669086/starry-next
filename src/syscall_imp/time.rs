use arceos_posix_api as api;
use axtask::{current, TaskExtRef, Tms};

pub(crate) fn sys_clock_gettime(clock_id: i32, tp: *mut api::ctypes::timespec) -> i32 {
    unsafe { api::sys_clock_gettime(clock_id, tp) }
}

pub(crate) fn sys_get_time_of_day(tv: *mut api::ctypes::timeval) -> i32 {
    unsafe { api::sys_get_time_of_day(tv) }
}

pub(crate) fn sys_times(tms: *mut Tms) -> isize {
    let curr = current();
    let children = curr.task_ext().children.lock();
    let res = curr.sys_times(&children);
    unsafe {
        tms.write(res);
    }
    res.tms_utime
}