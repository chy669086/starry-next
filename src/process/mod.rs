mod api;
pub mod signal;

use crate::flag::CloneFlags;
use crate::process::signal::SignalModule;
use crate::task::{read_trap_frame_from_kstack, TaskExt};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
pub use api::*;
use axerrno::AxResult;
use axhal::arch::UspaceContext;
use axhal::paging::MappingFlags;
use axmm::AddrSpace;
use axsync::Mutex;
use axtask::{current, yield_now, AxTaskRef, TaskExtRef, TaskInner};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use memory_addr::{MemoryAddr, VirtAddr};

pub type AxProcessRef = Arc<Process>;

pub struct Process {
    /// 进程 ID
    pub pid: u64,
    /// 父进程 ID
    pub ppid: AtomicU64,
    /// 子进程
    pub children: Mutex<Vec<AxProcessRef>>,
    /// 线程，tid -> thread
    pub threads: Mutex<BTreeMap<u64, AxTaskRef>>,
    /// 地址空间
    pub aspace: Arc<Mutex<AddrSpace>>,
    /// 退出码
    pub exit_code: AtomicI32,
    /// 堆底，用于 sbrk 系统调用
    pub heap_bottom: AtomicU64,
    /// 堆顶，用于 sbrk 系统调用
    pub heap_top: AtomicU64,
    /// 当前堆顶
    pub heap_current: AtomicU64,
    /// 进程状态
    pub is_exited: AtomicBool,
    /// 信号处理
    pub signal_module: Mutex<BTreeMap<u64, SignalModule>>,
}

const BRK_BOTTOM: u64 = 0x40000000;
const BRK_TOP: u64 = 0x80000000;

impl Process {
    pub fn new(ppid: u64, pid: u64, aspace: Arc<Mutex<AddrSpace>>) -> Self {
        Self {
            pid,
            ppid: AtomicU64::new(ppid),
            children: Mutex::new(Vec::new()),
            threads: Mutex::new(BTreeMap::new()),
            aspace,
            exit_code: AtomicI32::new(0),
            heap_bottom: AtomicU64::new(BRK_BOTTOM),
            heap_top: AtomicU64::new(BRK_TOP),
            heap_current: AtomicU64::new(BRK_BOTTOM),
            is_exited: AtomicBool::new(false),
            signal_module: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn state(&self) -> axtask::TaskState {
        if self.is_exited.load(Ordering::Relaxed) {
            axtask::TaskState::Exited
        } else {
            axtask::TaskState::Running
        }
    }

    pub fn main_thread(&self) -> AxTaskRef {
        self.threads.lock()[&self.pid].clone()
    }

    pub fn set_main_thread(&self, thread: AxTaskRef) {
        assert_eq!(thread.id().as_u64(), self.pid);
        self.add_thread(thread);
    }

    pub fn add_thread(&self, thread: AxTaskRef) {
        let tid = thread.id().as_u64();
        self.signal_module
            .lock()
            .insert(tid, SignalModule::new(None));
        self.threads.lock().insert(tid, thread);
    }

    pub fn is_main_thread(&self, thread: &AxTaskRef) -> bool {
        thread.id().as_u64() == self.pid
    }

    pub fn exit_thread(&self, thread: AxTaskRef, status: i32) {
        let tid = thread.id().as_u64();
        self.signal_module.lock().remove(&tid);
        // 主线程退出时，退出整个进程
        if self.is_main_thread(&thread) {
            self.exit(status);
            return;
        }
        let mut threads = self.threads.lock();
        let _thread = threads.remove(&tid).unwrap();
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Relaxed)
    }

    pub fn exit(&self, code: i32) {
        for child in self.children.lock().iter_mut() {
            child.ppid.store(1, Ordering::SeqCst);
        }
        self.is_exited.store(true, Ordering::Relaxed);

        // 等待其他线程退出
        // TODO: 直接退出其他线程
        while self.threads.lock().len() > 1 {
            yield_now();
        }

        self.exit_code.store(code, Ordering::Relaxed);
        remove_process(self.pid);
        debug!("Process {} exited with code {}", self.pid, code);
    }

    pub fn alloc_range_lazy(
        &self,
        start: VirtAddr,
        end: VirtAddr,
        flags: MappingFlags,
    ) -> AxResult<()> {
        if start > end {
            return Err(axerrno::AxError::InvalidInput);
        }
        let start = start.align_down_4k();
        let end = end.align_up_4k();
        let mut aspace = self.aspace.lock();
        aspace.map_alloc(start, end - start, flags, false)?;
        Ok(())
    }

    pub fn clone_proc(
        &self,
        flags: usize,
        stack: Option<usize>,
        _ptid: usize,
        _tls: usize,
        ctid: usize,
    ) -> AxResult<u64> {
        let clone_flags = CloneFlags::from_bits((flags & !0x3f) as u32).unwrap();

        // 对于 CLONE_THREAD，特殊处理
        if clone_flags.contains(CloneFlags::CLONE_THREAD) {
            return self.clone_thread(flags, stack, _ptid, _tls, ctid);
        }

        let curr = current();
        let mut trap_frame =
            read_trap_frame_from_kstack(curr.kernel_stack_top().unwrap().as_usize());

        let new_aspace = if clone_flags.contains(CloneFlags::CLONE_VM) {
            self.aspace.clone()
        } else {
            // TODO: 现有的复制方式似乎会破坏原有进程的空间，需要进一步优化，现在用共享空间代替
            // let new_aspace = AddrSpace::from_exited_space(&self.aspace.lock())?;
            // Arc::new(Mutex::new(new_aspace))
            self.aspace.clone()
        };

        let mut new_task = new_task();

        let pid = new_task.id().as_u64();
        let proc = if clone_flags.contains(CloneFlags::CLONE_PARENT) {
            // 共享父进程
            let ppid = self.ppid.load(Ordering::Relaxed);
            let proc = new_process(ppid, pid, new_aspace.clone());
            // 将子进程加入父进程的子进程列表
            // 由于现有进程模型的限制，系统进程不会被加入到进程管理器中
            get_process(ppid).map(|p| p.children.lock().push(proc.clone()));
            proc
        } else {
            let proc = new_process(self.pid, pid, new_aspace.clone());
            self.children.lock().push(proc.clone());
            proc
        };

        let page_root = new_aspace.lock().page_table_root();
        new_task.ctx_mut().set_page_table_root(page_root);

        trap_frame.regs.a0 = 0;
        trap_frame.sepc += 4;

        if let Some(stack) = stack {
            trap_frame.regs.sp = stack;
        }

        let new_uctx = UspaceContext::from(&trap_frame);

        let new_task_ext = TaskExt::new(new_uctx, &proc);

        // 共享文件描述符
        if clone_flags.contains(CloneFlags::CLONE_FILES) {
            new_task_ext.init_fs_shared()
        }

        if clone_flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) {
            new_task_ext.set_clear_child_tid(ctid as u64);
        }

        new_task_ext.init_ns();
        new_task.init_task_ext(new_task_ext);

        let new_task_ref = axtask::spawn_task(new_task);
        proc.set_main_thread(new_task_ref);

        Ok(pid)
    }

    // 对于 CLONE_THREAD，特殊处理
    pub fn clone_thread(
        &self,
        flags: usize,
        stack: Option<usize>,
        _ptid: usize,
        _tls: usize,
        ctid: usize,
    ) -> AxResult<u64> {
        let clone_flags = CloneFlags::from_bits((flags & !0x3f) as u32).unwrap();
        assert!(clone_flags.contains(CloneFlags::CLONE_THREAD));

        let mut new_task = new_task();

        let curr_task = current();
        let proc = curr_task.task_ext().get_proc().unwrap();

        let mut trap_frame =
            read_trap_frame_from_kstack(curr_task.kernel_stack_top().unwrap().as_usize());

        trap_frame.regs.a0 = 0;
        trap_frame.sepc += 4;

        if let Some(stack) = stack {
            trap_frame.regs.sp = stack;
        }

        let new_uctx = UspaceContext::from(&trap_frame);
        let new_task_ext = TaskExt::new(new_uctx, &proc);
        new_task_ext.init_fs_shared();

        if clone_flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) {
            new_task_ext.set_clear_child_tid(ctid as u64);
        }

        new_task_ext.init_ns();
        new_task.init_task_ext(new_task_ext);

        let new_task_ref = axtask::spawn_task(new_task);
        proc.add_thread(new_task_ref);

        Ok(proc.pid)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        info!("Process {} dropped", self.pid);
    }
}

fn new_task() -> TaskInner {
    TaskInner::new(
        || {
            let curr = current();
            let kstack_top = curr.kernel_stack_top().unwrap();
            info!(
                "Enter user space: entry={:#x}, ustack={:#x}, kstack={:#x}",
                curr.task_ext().uctx.get_ip(),
                curr.task_ext().uctx.get_sp(),
                kstack_top,
            );
            unsafe { curr.task_ext().uctx.enter_uspace(kstack_top) };
        },
        String::from(current().id_name()),
        crate::config::KERNEL_STACK_SIZE,
    )
}
