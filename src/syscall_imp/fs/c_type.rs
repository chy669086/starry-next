#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct DirEnt {
    pub d_ino: u64,
    pub d_off: i64,
    pub d_reclen: u16,
    pub d_type: u8,
    pub d_name: [u8; 0],
}

#[allow(dead_code)]
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum FileType {
    Unknown = 0,
    Fifo = 1,
    Chr = 2,
    Dir = 4,
    Blk = 6,
    Reg = 8,
    Lnk = 10,
    Socket = 12,
    Wht = 14,
}

impl From<axfs::api::FileType> for FileType {
    fn from(ft: axfs::api::FileType) -> Self {
        match ft {
            ft if ft.is_dir() => FileType::Dir,
            ft if ft.is_file() => FileType::Reg,
            _ => FileType::Unknown,
        }
    }
}

pub(crate) const DIR_ENT_SIZE: usize = core::mem::size_of::<u64>()
    + core::mem::size_of::<i64>()
    + core::mem::size_of::<u16>()
    + core::mem::size_of::<u8>();

impl DirEnt {
    pub(crate) fn new(ino: u64, off: i64, reclen: usize, file_type: FileType) -> Self {
        Self {
            d_ino: ino,
            d_off: off,
            d_reclen: reclen as u16,
            d_type: file_type as u8,
            d_name: [],
        }
    }
}

pub(crate) struct DirBuffer<'a> {
    buf: &'a mut [u8],
    offset: usize,
}

impl<'a> DirBuffer<'a> {
    pub(crate) unsafe fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, offset: 0 }
    }

    pub(crate) fn fit(&self, entry_size: usize) -> bool {
        self.offset + entry_size <= self.buf.len()
    }

    pub(crate) unsafe fn write(&mut self, dirent: DirEnt, name: &[u8]) -> Result<(), ()> {
        let entry_size = dirent.d_reclen as usize;
        if !self.fit(entry_size) {
            return Err(());
        }

        let buf_ptr = self.buf.as_mut_ptr().add(self.offset) as *mut DirEnt;
        buf_ptr.write(dirent);

        let name_ptr = buf_ptr.cast::<u8>().add(DIR_ENT_SIZE);
        name_ptr.copy_from(name.as_ptr(), name.len());

        self.offset += entry_size;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
#[cfg(not(target_arch = "x86_64"))]
pub struct Kstat {
    /// 设备
    pub st_dev: u64,
    /// inode 编号
    pub st_ino: u64,
    /// 文件类型
    pub st_mode: u32,
    /// 硬链接数
    pub st_nlink: u32,
    /// 用户id
    pub st_uid: u32,
    /// 用户组id
    pub st_gid: u32,
    /// 设备号
    pub st_rdev: u64,
    /// padding
    pub _pad0: u64,
    /// 文件大小
    pub st_size: u64,
    /// 块大小
    pub st_blksize: u32,
    /// padding
    pub _pad1: u32,
    /// 块个数
    pub st_blocks: u64,
    /// 最后一次访问时间(秒)
    pub st_atime_sec: isize,
    /// 最后一次访问时间(纳秒)
    pub st_atime_nsec: isize,
    /// 最后一次修改时间(秒)
    pub st_mtime_sec: isize,
    /// 最后一次修改时间(纳秒)
    pub st_mtime_nsec: isize,
    /// 最后一次改变状态时间(秒)
    pub st_ctime_sec: isize,
    /// 最后一次改变状态时间(纳秒)
    pub st_ctime_nsec: isize,
}

impl From<arceos_posix_api::ctypes::stat> for Kstat {
    fn from(stat: arceos_posix_api::ctypes::stat) -> Self {
        Self {
            st_dev: stat.st_dev,
            st_ino: stat.st_ino,
            st_mode: stat.st_mode,
            st_nlink: stat.st_nlink,
            st_uid: stat.st_uid,
            st_gid: stat.st_gid,
            st_rdev: stat.st_rdev,
            _pad0: 0,
            st_size: stat.st_size as u64,
            st_blksize: stat.st_blksize as u32,
            _pad1: 0,
            st_blocks: stat.st_blocks as u64,
            st_atime_sec: stat.st_atime.tv_sec as isize,
            st_atime_nsec: stat.st_atime.tv_nsec as isize,
            st_mtime_sec: stat.st_mtime.tv_sec as isize,
            st_mtime_nsec: stat.st_mtime.tv_nsec as isize,
            st_ctime_sec: stat.st_ctime.tv_sec as isize,
            st_ctime_nsec: stat.st_ctime.tv_nsec as isize,
        }
    }
}
