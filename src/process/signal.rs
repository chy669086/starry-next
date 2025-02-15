use crate::process::get_process;
use crate::signal::action::{SigActionFlags, SignalDefault, SIG_DFL, SIG_IGN};
use crate::signal::info::SigInfo;
use crate::signal::signal_no::SignalNo;
use crate::signal::ucontext::{SignalStack, SignalUserContext};
use crate::signal::{SignalHandler, SignalSet};
use crate::syscall_imp::sys_exit;
use crate::task::{read_trap_frame_from_kstack, write_trap_frame_to_kstack};
use alloc::sync::Arc;
use axerrno::AxResult;
use axhal::arch::TrapFrame;
use axhal::paging::MappingFlags;
use axstd::os::arceos::modules::axconfig;
use axsync::Mutex;
use axtask::{current, TaskExtRef};
use core::sync::atomic::Ordering;
use linkme::distributed_slice;

const USER_SIGNAL_PROTECT: usize = 512;

pub struct SignalModule {
    pub sig_info: bool,
    pub last_trap_frame: Option<TrapFrame>,
    pub sig_handler: Arc<Mutex<SignalHandler>>,
    pub sig_set: SignalSet,
    exit_sig: Option<SignalNo>,
    pub stack: SignalStack,
}

impl SignalModule {
    pub fn new(handler: Option<Arc<Mutex<SignalHandler>>>) -> Self {
        let sig_handler = handler.unwrap_or_else(|| Arc::new(Mutex::new(SignalHandler::new())));
        let sig_set = SignalSet::new();
        let last_trap_frame = None;
        let sig_info = false;
        Self {
            sig_info,
            last_trap_frame,
            sig_handler,
            sig_set,
            exit_sig: None,
            stack: SignalStack::default(),
        }
    }

    pub fn have_restart_signal(&self) -> Option<bool> {
        self.sig_set
            .find_sig()
            .map(|sig_num| self.sig_handler.lock().get_action(sig_num).need_restart())
    }

    pub fn set_exit_signal(&mut self, sig_num: SignalNo) {
        self.exit_sig = Some(sig_num);
    }

    pub fn get_exit_signal(&self) -> Option<SignalNo> {
        self.exit_sig
    }
}

#[no_mangle]
pub fn load_trap_for_signal() -> bool {
    let task = current();
    unsafe {
        if task.task_ext_ptr().is_null() {
            return false;
        }
    }
    let Some(proc) = task.task_ext().get_proc() else {
        return false;
    };

    let mut sig_modules = proc.signal_module.lock();
    let sig_module = sig_modules.get_mut(&task.id().as_u64()).unwrap();
    if let Some(old_trap_frame) = sig_module.last_trap_frame {
        let mut now_trap_frame =
            read_trap_frame_from_kstack(task.kernel_stack_top().unwrap().as_usize());
        let sp = now_trap_frame.regs.sp;
        now_trap_frame = old_trap_frame;
        if sig_module.sig_info {
            let pc = unsafe { (*(sp as *const SignalUserContext)).get_pc() };
            now_trap_frame.sepc = pc;
        }
        write_trap_frame_to_kstack(task.kernel_stack_top().unwrap().as_usize(), now_trap_frame);
        true
    } else {
        false
    }
}

#[distributed_slice(axhal::arch::HANDLE_SIGNAL)]
pub fn handle_signals() {
    let task = current();
    if unsafe { task.task_ext_ptr().is_null() } {
        // 只有系统进程才会没有 task_ext_ptr
        // 系统进程不会收到信号，所以不需要处理
        return;
    }
    let proc = task.task_ext().get_proc().unwrap();
    if proc.is_exited.load(Ordering::Relaxed) {
        // 进程已经退出，不再处理信号
        sys_exit(0);
    }
    let mut sig_modules = proc.signal_module.lock();

    let sig_module = sig_modules.get_mut(&task.id().as_u64()).unwrap();
    let sig_set = &mut sig_module.sig_set;
    let sig_num = if let Some(sig_num) = sig_set.get_one_sig() {
        sig_num
    } else {
        return;
    };

    let signal = SignalNo::from(sig_num);
    let mask = sig_set.mask;

    if sig_module.last_trap_frame.is_some() {
        // 之前的信号处理还没有完成
        // 产生了信号嵌套
        if signal == SignalNo::SIGSEGV || signal == SignalNo::SIGBUS {
            // 在处理信号的过程中又触发 SIGSEGV 或 SIGBUS，此时会导致死循环，所以直接结束当前进程
            drop(sig_modules);
            sys_exit(-1);
        }
        return;
    }

    // 保存当前的 trap frame
    sig_module.last_trap_frame = Some(read_trap_frame_from_kstack(
        task.kernel_stack_top().unwrap().as_usize(),
    ));

    sig_module.sig_info = false;

    // 处理信号
    let sig_handler = sig_module.sig_handler.lock();
    let action = sig_handler.get_action(sig_num).clone();
    if action.sa_handler == SIG_DFL {
        drop(sig_handler);
        drop(sig_modules);
        match SignalDefault::get_action(signal) {
            SignalDefault::Ignore => {
                // 忽略，此时相当于已经完成了处理，所以要把trap上下文清空
                load_trap_for_signal();
            }
            SignalDefault::Terminate => {
                terminate_process(signal, None);
            }
            SignalDefault::Stop => {
                unimplemented!();
            }
            SignalDefault::Cont => {
                unimplemented!();
            }
            SignalDefault::Core => {
                terminate_process(signal, None);
            }
        }
        return;
    }
    if action.sa_handler == SIG_IGN {
        // 忽略处理
        return;
    }

    let mut trap_frame = read_trap_frame_from_kstack(task.kernel_stack_top().unwrap().as_usize());

    let mut sp = if action.sa_flags.contains(SigActionFlags::SA_ONSTACK)
        && sig_module.stack.flags != crate::signal::ucontext::SS_DISABLE
    {
        debug!("Use alternate stack");
        (sig_module.stack.sp + sig_module.stack.size - 1) & !0xf
    } else {
        trap_frame.regs.sp - USER_SIGNAL_PROTECT
    };

    debug!("user signal stack: {:#x}", sp);

    let restorer = if let Some(addr) = action.get_storer() {
        addr
    } else {
        axconfig::SIGNAL_TRAMPOLINE
    };

    debug!(
        "restorer: {:#x}, handler: {:#x}",
        restorer, action.sa_handler
    );

    let old_pc = trap_frame.sepc;

    trap_frame.sepc = action.sa_handler;
    trap_frame.regs.a0 = sig_num;
    if action.sa_flags.contains(SigActionFlags::SA_SIGINFO) {
        sig_module.sig_info = true;
        let sp_base = (((sp - core::mem::size_of::<SigInfo>()) & !0xf)
            - core::mem::size_of::<SignalUserContext>())
            & !0xf;

        proc.alloc_range_lazy(sp_base.into(), sp.into(), MappingFlags::all())
            .expect("failed to alloc signal stack");

        sp = (sp - core::mem::size_of::<SigInfo>()) & !0xf;
        let info = if let Some(info) = sig_set.info.get(&(sig_num - 1)) {
            info!("test SigInfo: {:?}", info.0.si_val_int);
            info.0
        } else {
            SigInfo {
                si_signo: sig_num as i32,
                ..Default::default()
            }
        };
        unsafe {
            *(sp as *mut SigInfo) = info;
        }
        trap_frame.regs.a1 = sp;

        sp = (sp - core::mem::size_of::<SignalUserContext>()) & !0xf;

        let ucontext = SignalUserContext::init(old_pc, mask);
        unsafe {
            *(sp as *mut SignalUserContext) = ucontext;
        }
        trap_frame.regs.a2 = sp;
    }

    trap_frame.regs.sp = sp;

    write_trap_frame_to_kstack(task.kernel_stack_top().unwrap().as_usize(), trap_frame);
    drop(sig_handler);
    drop(sig_modules);
}

pub fn signal_return() -> isize {
    if load_trap_for_signal() {
        read_trap_frame_from_kstack(current().kernel_stack_top().unwrap().as_usize()).arg0()
            as isize
    } else {
        -1
    }
}

fn terminate_process(signal: SignalNo, info: Option<SigInfo>) {
    let task = current();
    let proc = task.task_ext().get_proc().unwrap();
    warn!("Terminate process: {}", proc.pid);
    if proc.is_main_thread(task.as_task_ref()) {
        sys_exit(signal as i32)
    } else {
        send_signal_to_proc(proc.pid, signal as isize, info).unwrap();
        sys_exit(-1)
    }
}

pub fn send_signal_to_proc(pid: u64, signal: isize, info: Option<SigInfo>) -> AxResult<()> {
    let Some(proc) = get_process(pid) else {
        return Err(axerrno::AxError::NotFound);
    };
    let main_thread = proc.main_thread();
    let mut sig_modules = proc.signal_module.lock();
    let sig_module = sig_modules.get_mut(&main_thread.id().as_u64()).unwrap();
    sig_module.sig_set.try_add_sig(signal as usize, info);
    // TODO: 如果主线程休眠，则唤醒处理信号
    Ok(())
}
