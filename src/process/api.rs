use crate::flag::WaitStatus;
use crate::process::{AxProcessRef, Process};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use axsync::Mutex;
use axtask::{current, AxTaskRef, TaskExtRef};
use lazy_static::lazy_static;

struct ProcessManager {
    inner: Mutex<ProcessManagerInner>,
}

struct ProcessManagerInner {
    processes: BTreeMap<u64, AxProcessRef>,
}

lazy_static! {
    static ref PID2PROC: ProcessManager = ProcessManager::new();
}

impl ProcessManagerInner {
    fn new() -> Self {
        Self {
            processes: BTreeMap::new(),
        }
    }

    fn new_process(&mut self, ppid: u64, task: AxTaskRef) -> AxProcessRef {
        let process = Arc::new(Process::from_task(task, ppid));
        let pid = process.pid;
        self.processes.insert(pid, process.clone());
        process
    }

    fn get_process(&self, pid: u64) -> Option<AxProcessRef> {
        self.processes.get(&pid).cloned()
    }

    fn remove_process(&mut self, pid: u64) {
        self.processes.remove(&pid);
    }
}

impl ProcessManager {
    fn new() -> Self {
        Self {
            inner: Mutex::new(ProcessManagerInner::new()),
        }
    }
}

pub fn remove_process(pid: u64) {
    let mut inner = PID2PROC.inner.lock();
    inner.remove_process(pid);
}

pub fn new_process(ppid: u64, task: AxTaskRef) -> AxProcessRef {
    let mut inner = PID2PROC.inner.lock();
    inner.new_process(ppid, task)
}

pub fn get_process(pid: u64) -> Option<AxProcessRef> {
    let inner = PID2PROC.inner.lock();
    inner.get_process(pid)
}

pub(crate) fn wait_pid(pid: i32, exit_code_ptr: *mut i32, _option: u32) -> Result<u64, WaitStatus> {
    if pid <= 0 {
        return wait_pid_negative(pid, exit_code_ptr, _option);
    }

    let curr_task = current();
    let proc = curr_task.task_ext().get_proc().unwrap();
    let mut proc_status = WaitStatus::NotExist;

    let child = proc
        .children
        .lock()
        .iter()
        .enumerate()
        .find(|(_id, child)| child.pid as i32 == pid)
        .map(|(id, child)| (id, child.clone()));

    let Some((loc, child)) = child else {
        return Err(WaitStatus::NotExist);
    };

    let state = child.state();
    if state == axtask::TaskState::Running {
        proc_status = WaitStatus::Running;
    } else if state == axtask::TaskState::Exited {
        let exit_code = child.exit_code();

        if !exit_code_ptr.is_null() {
            unsafe {
                *exit_code_ptr = exit_code << 8;
            }
        }

        let child_task = proc.children.lock().remove(loc);
        curr_task.add_child_time(&child_task.main_thread());

        return Ok(child_task.pid);
    }

    Err(proc_status)
}

fn wait_pid_negative(pid: i32, exit_code_ptr: *mut i32, _option: u32) -> Result<u64, WaitStatus> {
    assert!(pid <= 0);

    if pid == 0 {
        warn!("wait process group is not implemented");
    }

    let curr_task = current();
    let proc = curr_task.task_ext().get_proc().unwrap();
    let mut proc_status = WaitStatus::NotExist;
    let mut child_id = 0;

    for (id, task) in proc.children.lock().iter().enumerate() {
        proc_status = WaitStatus::Running;
        if task.state() == axtask::TaskState::Exited {
            proc_status = WaitStatus::Exited;
            child_id = id;
            break;
        }
    }

    if proc_status == WaitStatus::Exited {
        let child = proc.children.lock().remove(child_id);
        curr_task.add_child_time(&child.main_thread());

        let exit_code = child.exit_code();
        if !exit_code_ptr.is_null() {
            unsafe {
                *exit_code_ptr = exit_code << 8;
            }
        }

        return Ok(child.pid);
    }

    Err(proc_status)
}
