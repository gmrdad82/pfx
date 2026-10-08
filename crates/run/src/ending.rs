pub fn parent(stat: &str) -> Option<u32> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
pub use linux::{Process, below, terminate};

#[cfg(target_os = "linux")]
mod linux {
    use super::parent;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    pub struct Process(OwnedFd);

    impl Process {
        pub fn end(&self) {
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    self.0.as_raw_fd(),
                    libc::SIGTERM,
                    std::ptr::null::<libc::siginfo_t>(),
                    0u32,
                );
            }
        }
    }

    pub fn terminate(child: &mut std::process::Child) {
        if let Ok(pid) = libc::pid_t::try_from(child.id()) {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }

    fn open(pid: u32) -> Option<Process> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0u32) };
        (fd >= 0).then(|| Process(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
    }

    pub fn below(root: u32) -> Vec<Process> {
        let parents: Vec<(u32, u32)> = std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let pid = entry.file_name().to_str()?.parse().ok()?;
                let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
                Some((pid, parent(&stat)?))
            })
            .collect();
        let mut tree = vec![root];
        let mut at = 0;
        while at < tree.len() {
            let from = tree[at];
            tree.extend(parents.iter().filter(|(_, p)| *p == from).map(|(c, _)| *c));
            at += 1;
        }
        tree.into_iter().skip(1).filter_map(open).collect()
    }
}

#[cfg(not(target_os = "linux"))]
pub struct Process;

#[cfg(not(target_os = "linux"))]
impl Process {
    pub fn end(&self) {}
}

#[cfg(not(target_os = "linux"))]
pub fn below(_root: u32) -> Vec<Process> {
    Vec::new()
}

#[cfg(not(target_os = "linux"))]
pub fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
}

use std::sync::atomic::{AtomicI32, Ordering};

static STOP: AtomicI32 = AtomicI32::new(0);
#[cfg(target_os = "linux")]
static QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
static CHILDREN: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

pub struct Tracked {
    #[cfg(target_os = "linux")]
    pid: i32,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let mut kids = CHILDREN.lock().unwrap_or_else(|poison| poison.into_inner());
            if let Some(at) = kids.iter().position(|pid| *pid == self.pid) {
                kids.remove(at);
            }
        }
    }
}

pub fn track(child: &std::process::Child) -> Tracked {
    #[cfg(target_os = "linux")]
    {
        let pid = rustix::process::Pid::from_child(child).as_raw_pid();
        CHILDREN
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(pid);
        Tracked { pid }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = child;
        Tracked {}
    }
}

pub fn stopped() -> Option<i32> {
    let signal = STOP.load(Ordering::SeqCst);
    (signal != 0).then_some(signal)
}

pub fn stop_like(signal: i32) -> ! {
    #[cfg(target_os = "linux")]
    {
        if QUIT.swap(true, Ordering::SeqCst) {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        let _ = signal_hook::low_level::emulate_default_handler(signal);
        signal_hook::low_level::exit(128 + signal);
    }
    #[cfg(not(target_os = "linux"))]
    std::process::exit(128 + signal);
}

pub fn install() -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        let mut result: Result<(), String> = Ok(());
        ONCE.call_once(|| {
            result = arm();
        });
        result
    }
}

#[cfg(target_os = "linux")]
fn arm() -> Result<(), String> {
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ])
    .map_err(|error| format!("signals: {error}"))?;
    std::thread::Builder::new()
        .name("pfx-signals".into())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                STOP.store(signal, Ordering::SeqCst);
                pass_term();
                stop_like(signal);
            }
        })
        .map_err(|error| format!("signals: {error}"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn pass_term() {
    let pids = CHILDREN
        .lock()
        .map(|kids| kids.clone())
        .unwrap_or_else(|poison| poison.into_inner().clone());
    for pid in &pids {
        if let Some(pid) = rustix::process::Pid::from_raw(*pid) {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let mut alive = false;
        for pid in &pids {
            let Some(pid) = rustix::process::Pid::from_raw(*pid) else {
                continue;
            };
            if matches!(
                rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
                Ok(None)
            ) {
                alive = true;
            }
        }
        if !alive {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_is_read_past_a_name_with_spaces_and_brackets() {
        assert_eq!(parent("4242 (papp (x) y) S 17 4242 17 0 -1"), Some(17));
        assert_eq!(parent("4242 (blender (x) y) S 17 4242 17 0 -1"), Some(17));
        assert_eq!(parent("1 (systemd) S 0 1 1 0 -1"), Some(0));
        assert_eq!(parent("garbage"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_processes_below_a_wrapper_end_and_the_wrapper_lives_to_report_it() {
        use std::time::{Duration, Instant};
        let mut wrapper = std::process::Command::new("sh")
            .args(["-c", "sleep 60 & wait $!"])
            .spawn()
            .unwrap();
        let asked = Instant::now();
        let mut found = below(wrapper.id());
        while found.is_empty() && asked.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
            found = below(wrapper.id());
        }
        assert_eq!(found.len(), 1);
        found.iter().for_each(Process::end);
        let status = wrapper.wait().unwrap();
        assert!(asked.elapsed() < Duration::from_secs(30));
        assert_eq!(status.code(), Some(128 + 15));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_is_asked_to_end_with_term() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        terminate(&mut child);
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGTERM));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_tracked_child_is_asked_to_end() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("tracked-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let got = dir.join("got");
        let ready = dir.join("ready");
        let mut child = std::process::Command::new("/usr/bin/python3")
            .arg("-c")
            .arg(concat!(
                "import signal,sys,time\n",
                "got,ready=sys.argv[1],sys.argv[2]\n",
                "def stop(signum,frame):\n",
                "    open(got,'w').write('term')\n",
                "    sys.exit(0)\n",
                "signal.signal(signal.SIGTERM, stop)\n",
                "open(ready,'w').write('ready')\n",
                "while True:\n",
                "    time.sleep(30)\n",
            ))
            .arg(&got)
            .arg(&ready)
            .spawn()
            .unwrap();
        let tracked = track(&child);
        let asked = std::time::Instant::now();
        while !ready.exists() && asked.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready.exists());
        pass_term();
        drop(tracked);
        let _ = child.try_wait();
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "term");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
