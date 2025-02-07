use alloc::string::ToString;
use arceos_posix_api as api;
use core::ffi::{c_char, c_void};

use crate::syscall_body;
use crate::syscall_imp::fs::c_type::{DirBuffer, DirEnt, FileType, Kstat, DIR_ENT_SIZE};

/// The ioctl() system call manipulates the underlying device parameters
/// of special files.
///
/// # Arguments
/// * `fd` - The file descriptor
/// * `op` - The request code. It is of type unsigned long in glibc and BSD,
/// and of type int in musl and other UNIX systems.
/// * `argp` - The argument to the request. It is a pointer to a memory location
pub(crate) fn sys_ioctl(_fd: i32, _op: usize, _argp: *mut c_void) -> i32 {
    syscall_body!(sys_ioctl, {
        warn!("Unimplemented syscall: SYS_IOCTL");
        Ok(0)
    })
}

pub(crate) fn sys_getdents64(fd: i32, buf: *mut c_void, len: usize) -> i32 {
    syscall_body!(sys_getdent64, {
        if len < DIR_ENT_SIZE {
            return Err(axerrno::LinuxError::EPERM);
        }

        let path = api::Directory::from_fd(fd).map(|dir| dir.path().to_string())?;

        let mut buffer =
            unsafe { DirBuffer::new(core::slice::from_raw_parts_mut(buf as *mut u8, len)) };

        axfs::api::read_dir(&path)
            .map_err(Into::into)
            .and_then(|entries| {
                let mut offset = 0;
                for entry in entries.flatten() {
                    let mut name = entry.file_name();
                    name.push('\0');

                    let entry_size = name.len() + DIR_ENT_SIZE;
                    offset += entry_size;

                    let dirent =
                        DirEnt::new(1, offset as i64, entry_size, entry.file_type().into());

                    unsafe {
                        if buffer.write(dirent, name.as_bytes()).is_err() {
                            break;
                        }
                    }
                }
                if offset > 0 && buffer.fit(DIR_ENT_SIZE) {
                    let terminal = DirEnt::new(1, offset as i64, 0, FileType::Reg);
                    unsafe {
                        let _ = buffer.write(terminal, &[]);
                    }
                }

                Ok(offset as isize)
            })
    })
}

/// Because the FAT32 file system does not support the `link` system call,
/// So we just return -1.
pub(crate) fn sys_linkat(
    _old_dirfd: i32,
    _old_path: *const c_void,
    _new_dirfd: i32,
    _new_path: *const c_void,
    flags: i32,
) -> i32 {
    if flags != 0 {
        warn!("Unsupport flags: {}", flags);
    }

    -1
}

pub(crate) fn sys_unlinkat(dirfd: i32, pathname: *const c_char, flags: i32) -> i32 {
    if flags != 0 {
        warn!("Unsupport flags: {}", flags);
    }
    api::sys_unlinkat(dirfd, pathname, flags)
}

pub(crate) fn sys_fstat(fd: i32, statbuf: *mut c_void) -> i32 {
    let kstat_ptr = statbuf as *mut Kstat;
    let mut stat = api::ctypes::stat::default();
    let ret = unsafe { api::sys_fstat(fd, &mut stat) };
    if ret < 0 {
        return -1;
    }
    let kstat = Kstat::from(stat);
    unsafe {
        kstat_ptr.write(kstat);
    }
    0
}

pub(crate) fn sys_lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    api::sys_lseek(fd, offset, whence)
}
