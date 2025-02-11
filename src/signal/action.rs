use super::signal_no::SignalNo::*;
use crate::signal::signal_no::SignalNo;
use bitflags::bitflags;

/// 特殊取值，代表默认处理函数
pub const SIG_DFL: usize = 0;

/// 特殊取值，代表忽略这个信号
pub const SIG_IGN: usize = 1;

bitflags! {
    #[allow(missing_docs)]
    #[derive(Default,Clone, Copy, Debug)]
    pub struct SigActionFlags: u32 {
        /// 当子进程停止时，停止收到通知
        const SA_NOCLDSTOP = 1;
        /// 当子进程终止时，不创建僵尸进程
        const SA_NOCLDWAIT = 2;
        /// 使用具有三个参数的信号处理程序，需要设置 `sa_sigaction` 而不是 `sa_handler`
        const SA_SIGINFO = 4;
        /// 信号处理程序的栈是使用 `sigaltstack` 设置的
        const SA_ONSTACK = 0x08000000;
        /// 重新启动系统调用
        const SA_RESTART = 0x10000000;
        /// 不屏蔽正在处理的信号
        const SA_NODEFER = 0x40000000;
        /// 处理完信号后恢复默认处理
        const SA_RESETHAND = 0x80000000;
        /// 使用自定义的信号恢复函数
        const SA_RESTORER = 0x04000000;
    }
}

/// 没有显式指定处理函数时的默认行为
pub enum SignalDefault {
    /// 终止进程
    Terminate,
    /// 忽略信号
    Ignore,
    /// 终止进程并转储核心，即程序当时的内存状态记录下来，保存在一个文件中，但当前未实现保存，直接退出进程
    Core,
    /// 暂停进程执行
    Stop,
    /// 恢复进程执行
    Cont,
}

impl SignalDefault {
    /// Get the default action of a signal
    pub fn get_action(signal: SignalNo) -> Self {
        match signal {
            SIGABRT => Self::Core,
            SIGALRM => Self::Terminate,
            SIGBUS => Self::Core,
            SIGCHLD => Self::Ignore,
            SIGCONT => Self::Cont,
            SIGFPE => Self::Core,
            SIGHUP => Self::Terminate,
            SIGILL => Self::Core,
            SIGINT => Self::Terminate,
            SIGKILL => Self::Terminate,
            SIGPIPE => Self::Terminate,
            SIGQUIT => Self::Core,
            SIGSEGV => Self::Core,
            SIGSTOP => Self::Stop,
            SIGTERM => Self::Terminate,
            SIGTSTP => Self::Stop,
            SIGTTIN => Self::Stop,
            SIGTTOU => Self::Stop,
            SIGUSR1 => Self::Terminate,
            SIGUSR2 => Self::Terminate,
            SIGXCPU => Self::Core,
            SIGXFSZ => Self::Core,
            SIGVTALRM => Self::Terminate,
            SIGPROF => Self::Terminate,
            SIGWINCH => Self::Ignore,
            SIGIO => Self::Terminate,
            SIGPWR => Self::Terminate,
            SIGSYS => Self::Core,
            _ => Self::Terminate,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SigAction {
    /// 信号处理函数地址
    pub sa_handler: usize,
    /// 信号处理标志
    pub sa_flags: SigActionFlags,
    /// 信号处理的跳板页地址，存储了sig_return的函数处理地址
    /// 仅在SA_RESTORER标志被设置时有效
    pub sa_restorer: usize,
    /// 该信号处理函数的信号掩码
    pub sa_mask: usize,
}

impl SigAction {
    /// get the restorer address of the signal action
    ///
    /// When the SA_RESTORER flag is set, the restorer address is valid
    ///
    /// or it will return None, and the core will set the restore address as the signal trampoline
    pub fn get_storer(&self) -> Option<usize> {
        if self.sa_flags.contains(SigActionFlags::SA_RESTORER) {
            Some(self.sa_restorer)
        } else {
            None
        }
    }

    /// Whether the syscall should be restarted after the signal handler returns
    pub fn need_restart(&self) -> bool {
        self.sa_flags.contains(SigActionFlags::SA_RESTART)
    }
}
