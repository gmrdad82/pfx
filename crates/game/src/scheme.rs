use std::sync::mpsc::Receiver;

use pfx_play::Scheme;

#[cfg(all(unix, not(target_os = "macos")))]
mod bus;

pub fn system_scheme() -> Option<Scheme> {
    read()
}

pub(crate) fn of_theme(theme: winit::window::Theme) -> Scheme {
    match theme {
        winit::window::Theme::Light => Scheme::Light,
        winit::window::Theme::Dark => Scheme::Dark,
    }
}

pub(crate) const FROM_WINDOW: bool = cfg!(any(windows, target_os = "macos"));

#[cfg(all(unix, not(target_os = "macos")))]
fn read() -> Option<Scheme> {
    bus::read()
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn watch(wake: impl Fn() + Send + 'static) -> Option<Receiver<Option<Scheme>>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    bus::watch(move |scheme| {
        let sent = sender.send(scheme).is_ok();
        wake();
        sent
    })
    .then_some(receiver)
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(crate) fn watch(_wake: impl Fn() + Send + 'static) -> Option<Receiver<Option<Scheme>>> {
    None
}

#[cfg(windows)]
fn read() -> Option<Scheme> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    (status == 0).then_some(if value == 0 {
        Scheme::Dark
    } else {
        Scheme::Light
    })
}

#[cfg(target_os = "macos")]
fn read() -> Option<Scheme> {
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .ok()?;
    let dark = out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "Dark";
    Some(if dark { Scheme::Dark } else { Scheme::Light })
}

#[cfg(not(any(unix, windows)))]
fn read() -> Option<Scheme> {
    None
}
