#[cfg(not(windows))]
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
#[cfg(not(windows))]
use std::time::{Duration, Instant};

use pfx_play::{Answer, Browser, Opening, Url, UrlError};

#[cfg(not(windows))]
const OPENER_WAIT: Duration = Duration::from_secs(2);
#[cfg(not(windows))]
const OPENER_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Default)]
pub struct SystemBrowser {
    busy: Arc<AtomicBool>,
}

impl Browser for SystemBrowser {
    fn open(&self, url: &Url) -> Opening {
        let (answer, opening) = Opening::pending();
        self.send(url, answer);
        opening
    }
}

impl SystemBrowser {
    fn send(&self, url: &Url, answer: Answer) {
        if self.busy.swap(true, Ordering::AcqRel) {
            let _ = answer.send(Err(UrlError::Busy));
            return;
        }
        let url = url.clone();
        let busy = self.busy.clone();
        let reply = answer.clone();
        let spawned = thread::Builder::new()
            .name("pfx browser".into())
            .spawn(move || {
                launch(&url, move |result| {
                    busy.store(false, Ordering::Release);
                    let _ = answer.send(result);
                });
            });
        if let Err(error) = spawned {
            self.busy.store(false, Ordering::Release);
            let _ = reply.send(Err(UrlError::Failed(format!(
                "the browser's thread: {error}"
            ))));
        }
    }
}

#[cfg(not(windows))]
fn launchers(url: &Url) -> Vec<Command> {
    let url = url.as_str();
    let mut commands = Vec::new();
    if cfg!(target_os = "macos") {
        let mut open = Command::new("/usr/bin/open");
        open.arg("--").arg(url);
        commands.push(open);
    } else if cfg!(all(unix, not(target_os = "macos"))) {
        let mut xdg = Command::new("xdg-open");
        xdg.arg(url);
        commands.push(xdg);
        let mut gio = Command::new("gio");
        gio.arg("open").arg(url);
        commands.push(gio);
        let mut gnome = Command::new("gnome-open");
        gnome.arg(url);
        commands.push(gnome);
        let mut kde = Command::new("kde-open");
        kde.arg("--").arg(url);
        commands.push(kde);
    }
    commands
}

#[cfg(windows)]
fn launch(url: &Url, answer: impl FnOnce(Result<(), UrlError>)) {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let file: Vec<u16> = url.as_str().encode_utf16().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    let com = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    let done = unsafe { ShellExecuteExW(&mut info) };
    let error = unsafe { GetLastError() };
    if com >= 0 {
        unsafe { CoUninitialize() };
    }
    if done != 0 {
        answer(Ok(()));
    } else {
        answer(Err(UrlError::Failed(format!(
            "ShellExecuteEx answered error {error}"
        ))));
    }
}

#[cfg(not(windows))]
fn launch(url: &Url, answer: impl FnOnce(Result<(), UrlError>)) {
    let mut missing = Vec::new();
    for mut command in launchers(url) {
        let name = command.get_program().to_string_lossy().into_owned();
        let spawned = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => return settle(&name, child, answer),
            Err(error) => missing.push(format!("{name}: {error}")),
        }
    }
    if missing.is_empty() {
        return answer(Err(UrlError::Failed(
            "no system browser on this platform".into(),
        )));
    }
    answer(Err(UrlError::Failed(missing.join("; "))));
}

#[cfg(not(windows))]
fn settle(name: &str, mut child: Child, answer: impl FnOnce(Result<(), UrlError>)) {
    let deadline = Instant::now() + OPENER_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return answer(Ok(())),
            Ok(Some(status)) => {
                return answer(Err(UrlError::Failed(format!("{name} answered {status}"))));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(OPENER_POLL),
            Ok(None) => break,
            Err(error) => return answer(Err(UrlError::Failed(format!("{name}: {error}")))),
        }
    }
    answer(Ok(()));
    let _ = child.wait();
}
