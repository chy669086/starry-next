
pub const SIGSET_SIZE_IN_BYTE: usize = 8;

/// sys_sigprocmask 中指定的结构体类型
pub enum SigMaskFlag {
    /// add the mask to the block mask
    Block = 0,
    /// unblock the mask from the block mask
    Unblock = 1,
    /// set the mask as the new block mask
    Setmask = 2,
}

impl SigMaskFlag {
    /// turn a usize to SigMaskFlag
    pub fn from(value: usize) -> Self {
        match value {
            0 => SigMaskFlag::Block,
            1 => SigMaskFlag::Unblock,
            2 => SigMaskFlag::Setmask,
            _ => panic!("SIG_MASK_FLAG::from: invalid value"),
        }
    }
}