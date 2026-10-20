pub const PRIORITY_VAR: &str = "PFX_UI_PRIORITY";
pub const RAISED_NICE: i32 = -10;

pub fn asked() -> bool {
    std::env::var(PRIORITY_VAR).is_ok_and(|value| named(&value))
}

pub fn named(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "raised" | "on" | "1"
    )
}

pub fn nice_floor(rlimit_nice: u64) -> i32 {
    let floor = 20i64 - i64::try_from(rlimit_nice).unwrap_or(i64::MAX);
    i32::try_from(floor.clamp(-20, 19)).unwrap_or(19)
}

pub fn nice_target(current: i32, rlimit_nice: u64, privileged: bool) -> Option<i32> {
    let floor = if privileged {
        RAISED_NICE
    } else {
        RAISED_NICE.max(nice_floor(rlimit_nice))
    };
    (floor < current).then_some(floor)
}

#[cfg(target_os = "linux")]
pub fn raise_current_thread() -> Result<String, String> {
    let tid = unsafe { libc::gettid() };
    let who = libc::id_t::try_from(tid).map_err(|error| error.to_string())?;
    let errno = std::io::Error::last_os_error;
    unsafe { *libc::__errno_location() = 0 };
    let current = unsafe { libc::getpriority(libc::PRIO_PROCESS as _, who) };
    if current == -1 && errno().raw_os_error().is_some_and(|code| code != 0) {
        return Err(format!("cannot read the thread's nice: {}", errno()));
    }
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NICE, &mut limit) } != 0 {
        return Err(format!("cannot read RLIMIT_NICE: {}", errno()));
    }
    let rlimit = if limit.rlim_cur == libc::RLIM_INFINITY {
        40
    } else {
        limit.rlim_cur
    };
    let privileged = unsafe { libc::geteuid() } == 0;
    let Some(target) = nice_target(current, rlimit, privileged) else {
        return Err(format!(
            "nice stays {current}: RLIMIT_NICE {rlimit} allows nothing lower"
        ));
    };
    if unsafe { libc::setpriority(libc::PRIO_PROCESS as _, who, target) } != 0 {
        return Err(format!("cannot set nice {target}: {}", errno()));
    }
    Ok(format!("nice {current} to {target}"))
}

#[cfg(target_os = "windows")]
pub fn raise_current_thread() -> Result<String, String> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    };
    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) } == 0 {
        return Err(format!(
            "cannot set THREAD_PRIORITY_ABOVE_NORMAL: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok("THREAD_PRIORITY_ABOVE_NORMAL".to_string())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn raise_current_thread() -> Result<String, String> {
    let code = unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0)
    };
    if code != 0 {
        return Err(format!(
            "cannot set QOS_CLASS_USER_INTERACTIVE: {}",
            std::io::Error::from_raw_os_error(code)
        ));
    }
    Ok("QOS_CLASS_USER_INTERACTIVE".to_string())
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    target_os = "ios"
)))]
pub fn raise_current_thread() -> Result<String, String> {
    Err("no thread priority on this platform".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nice_goes_no_lower_than_ten_and_never_past_the_limit() {
        assert_eq!(nice_floor(0), 19);
        assert_eq!(nice_floor(20), 0);
        assert_eq!(nice_floor(31), -11);
        assert_eq!(nice_floor(40), -20);
        assert_eq!(nice_target(0, 31, false), Some(-10));
        assert_eq!(nice_target(0, 25, false), Some(-5));
        assert_eq!(nice_target(0, 0, false), None);
        assert_eq!(nice_target(0, 0, true), Some(-10));
        assert_eq!(nice_target(-15, 40, false), None);
        assert!(named(" Raised") && named("on") && !named("off") && !named(""));
    }
}
