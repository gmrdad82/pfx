use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, GetKeyboardLayout, HKL, MAPVK_VK_TO_CHAR, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW,
};

use super::{LayoutSource, scancode};
use crate::device::Key;

const DEAD: u32 = 0x8000_0000;

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsLayout;

impl WindowsLayout {
    pub fn new() -> Self {
        Self
    }

    fn hkl() -> HKL {
        unsafe { GetKeyboardLayout(0) }
    }
}

impl LayoutSource for WindowsLayout {
    fn layout(&mut self) -> u64 {
        Self::hkl() as usize as u64
    }

    fn label(&mut self, key: Key) -> Option<String> {
        let code = u32::from(scancode(key)?);
        let hkl = Self::hkl();
        let vk = unsafe { MapVirtualKeyExW(code, MAPVK_VSC_TO_VK_EX, hkl) };
        if vk != 0 {
            let unit = unsafe { MapVirtualKeyExW(vk, MAPVK_VK_TO_CHAR, hkl) } & !DEAD;
            if let Some(c) = char::from_u32(unit).filter(|c| *c != '\0') {
                return Some(c.to_string());
            }
        }
        let mut name = [0u16; 32];
        let len = unsafe { GetKeyNameTextW((code << 16) as i32, name.as_mut_ptr(), 32) };
        let len = usize::try_from(len).ok().filter(|len| *len > 0)?;
        String::from_utf16(&name[..len.min(name.len())]).ok()
    }
}
