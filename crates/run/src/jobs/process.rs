use std::process::Command;

use super::state::Proc;

#[cfg(target_os = "linux")]
pub fn started(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if matches!(fields.first(), Some(&"Z") | Some(&"X")) {
        return None;
    }
    let ticks = fields.get(19)?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    Some(format!("{}:{ticks}", boot.trim()))
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn started(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "stat=,lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (state, start) = text.trim().split_once(char::is_whitespace)?;
    (!state.starts_with('Z')).then(|| start.trim().to_string())
}

#[cfg(not(unix))]
pub fn started(_pid: u32) -> Option<String> {
    None
}

pub fn me() -> Proc {
    of(std::process::id())
}

pub fn of(pid: u32) -> Proc {
    Proc {
        pid,
        start: started(pid),
    }
}

pub fn same(proc: &Proc) -> bool {
    match (&proc.start, started(proc.pid)) {
        (Some(was), Some(now)) => *was == now,
        _ => false,
    }
}

pub fn detach(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(not(windows))]
    let _ = command;
}

pub fn leave_session() {
    #[cfg(unix)]
    let _ = rustix::process::setsid();
}

pub fn own_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

pub fn end_group(pid: u32, hard: bool) {
    #[cfg(unix)]
    if let Some(group) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let signal = if hard {
            rustix::process::Signal::KILL
        } else {
            rustix::process::Signal::TERM
        };
        let _ = rustix::process::kill_process_group(group, signal);
    }
    #[cfg(not(unix))]
    let _ = (pid, hard);
}

pub fn reap(child: &Proc) -> bool {
    if !same(child) {
        return false;
    }
    end_group(child.pid, false);
    let asked = std::time::Instant::now();
    while same(child) && asked.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if same(child) {
        end_group(child.pid, true);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_process_is_known_by_its_start_and_a_dead_or_reused_pid_is_not_it() {
        let mine = me();
        assert!(mine.start.is_some());
        assert!(same(&mine));
        let reused = Proc {
            pid: mine.pid,
            start: Some("another boot:1".into()),
        };
        assert!(!same(&reused));
        assert!(!same(&Proc {
            pid: mine.pid,
            start: None
        }));
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let held = of(child.id());
        assert!(same(&held));
        assert!(held.start != mine.start);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!same(&held));
    }

    #[cfg(unix)]
    #[test]
    fn reaping_ends_the_whole_group_of_a_child_left_behind() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60 & wait"]);
        own_group(&mut command);
        let mut child = command.spawn().unwrap();
        let held = of(child.id());
        assert!(reap(&held));
        child.wait().unwrap();
        assert!(!same(&held));
        assert!(!reap(&held));
    }
}
