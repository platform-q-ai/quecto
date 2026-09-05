//! Linux exact-process ownership tracking. Numeric PGID liveness is not proof
//! of ownership after reap. The root must remain unreaped for this entire value's
//! lifetime; other descendants are validated against live parent pidfds.
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

#[derive(Debug)]
struct Process {
    pid: i32,
    parent: i32,
    group: i32,
    start: u64,
}
fn stat(pid: i32) -> Option<Process> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = text
        .get(text.rfind(')')? + 2..)?
        .split_whitespace()
        .collect();
    Some(Process {
        pid,
        parent: fields.get(1)?.parse().ok()?,
        group: fields.get(2)?.parse().ok()?,
        start: fields.get(19)?.parse().ok()?,
    })
}
#[derive(Debug)]
struct Pinned {
    start: u64,
    fd: OwnedFd,
}
impl Pinned {
    fn exited(&self) -> bool {
        let mut poll = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: valid owned pidfd, one valid pollfd, nonblocking timeout.
        unsafe { libc::poll(&mut poll, 1, 0) > 0 && poll.revents & libc::POLLIN != 0 }
    }
}
#[derive(Debug)]
pub(crate) struct OwnedProcesses {
    root: i32,
    tasks: HashMap<i32, Pinned>,
    certain: bool,
}
impl OwnedProcesses {
    pub(crate) fn new(root: i32) -> Self {
        Self {
            root,
            tasks: HashMap::new(),
            certain: true,
        }
    }
    pub(crate) fn refresh(&mut self) {
        // Retain exact handles until exit; removing completed handles permits a
        // newly created owned process with a reused PID to be tracked normally.
        self.tasks.retain(|_, task| !task.exited());
        let Ok(dir) = std::fs::read_dir("/proc") else {
            self.certain = false;
            return;
        };
        let mut pending: Vec<_> = dir
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
            .filter(|pid| *pid != self.root)
            .filter_map(stat)
            .collect();
        loop {
            let before = self.tasks.len();
            pending.retain(|process| {
                if self.tasks.contains_key(&process.pid) {
                    return false;
                }
                let owned = process.parent == self.root
                    || process.group == self.root
                    || self
                        .tasks
                        .get(&process.parent)
                        .is_some_and(|parent| !parent.exited());
                if !owned {
                    return true;
                }
                self.capture(process);
                false
            });
            if self.tasks.len() == before {
                break;
            }
        }
    }
    fn capture(&mut self, process: &Process) {
        // SAFETY: pidfd_open returns a handle to this exact process, not a future PID user.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, process.pid, 0) };
        if raw < 0 {
            if stat(process.pid).is_some() {
                self.certain = false;
            }
            return;
        }
        // SAFETY: a successful pidfd_open returned a fresh owned descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        let Some(now) = stat(process.pid) else {
            return;
        };
        if now.start != process.start || now.parent != process.parent || now.group != process.group
        {
            return;
        }
        // Validate the ancestry *after* pinning and re-reading the child. A live
        // parent pidfd cannot have been replaced by a recycled parent PID.
        let parent_owned = now.parent == self.root
            || now.group == self.root
            || self.tasks.get(&now.parent).is_some_and(|parent| {
                stat(now.parent).is_some_and(|p| p.start == parent.start) && !parent.exited()
            });
        if parent_owned {
            self.tasks.insert(
                now.pid,
                Pinned {
                    start: now.start,
                    fd,
                },
            );
        }
    }
    pub(crate) fn signal(&self, signal: i32) {
        for task in self.tasks.values() {
            // SAFETY: signal only an owned exact pidfd, never a numeric recycled PID/group.
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    task.fd.as_raw_fd(),
                    signal,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                );
            }
        }
    }
    pub(crate) fn all_exited(&self) -> bool {
        self.certain && self.tasks.values().all(Pinned::exited)
    }
}
