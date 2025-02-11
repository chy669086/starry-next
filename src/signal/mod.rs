use crate::signal::action::SigAction;
use crate::signal::info::SigInfo;
use crate::signal::signal_no::{SignalNo, MAX_SIG_NUM};
use crate::signal::ucontext::SignalUserContext;
use alloc::collections::BTreeMap;

pub mod action;
pub mod info;
pub mod signal_no;

pub mod ucontext;

#[derive(Clone)]
pub struct SignalHandler {
    pub handlers: [SigAction; MAX_SIG_NUM],
}

impl SignalHandler {
    pub fn new() -> Self {
        Self {
            handlers: [SigAction::default(); MAX_SIG_NUM],
        }
    }

    pub fn clear(&mut self) {
        for action in self.handlers.iter_mut() {
            *action = SigAction::default();
        }
    }

    pub fn get_action(&self, sig_num: usize) -> &SigAction {
        &self.handlers[sig_num - 1]
    }

    pub unsafe fn set_action(&mut self, sig_num: usize, action: *const SigAction) {
        self.handlers[sig_num - 1] = unsafe { *action };
    }
}

impl Default for SignalHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// 接受信号的结构，每一个进程都有一个
#[derive(Clone)]
pub struct SignalSet {
    /// 信号掩码
    pub mask: usize,
    /// 未决信号集
    pub pending: usize,
    /// 附加信息
    pub info: BTreeMap<usize, (SigInfo, SignalUserContext)>,
}

impl SignalSet {
    pub fn new() -> Self {
        Self {
            mask: 0,
            pending: 0,
            info: BTreeMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.mask = 0;
        self.pending = 0;
    }

    pub fn find_sig(&self) -> Option<usize> {
        let mut pending = self.pending;
        loop {
            let pos = pending.trailing_zeros();
            if pos == MAX_SIG_NUM as u32 {
                return None;
            }

            pending &= !(1 << pos);
            if self.mask & (1 << pos) == 0
                || pos == SignalNo::SIGKILL as u32 - 1
                || pos == SignalNo::SIGSTOP as u32 - 1
            {
                return Some(pos as usize + 1);
            }
        }
    }

    pub fn get_one_sig(&mut self) -> Option<usize> {
        if let Some(sig) = self.find_sig() {
            self.pending &= !(1 << (sig - 1));
            Some(sig)
        } else {
            None
        }
    }

    pub fn try_add_sig(&mut self, sig_num: usize, info: Option<SigInfo>) {
        let now_mask = 1 << (sig_num - 1);
        self.mask |= now_mask;
        if let Some(info) = info {
            self.info
                .insert(sig_num, (info, SignalUserContext::default()));
        }
    }
}
