#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
mod wayland;

use winit::window::Window;

use crate::presents::Shown;

pub struct Hint {
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    content: Option<wayland::ContentHint>,
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    feedback: Option<wayland::PresentationFeedback>,
    vrr: Option<bool>,
}

impl Hint {
    pub fn new(window: &Window) -> Self {
        let _ = window;
        Self {
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            content: wayland::ContentHint::new(window),
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            feedback: wayland::PresentationFeedback::new(window),
            vrr: None,
        }
    }

    pub fn set(&mut self, vrr: bool) {
        if self.vrr == Some(vrr) {
            return;
        }
        self.vrr = Some(vrr);
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd"
        ))]
        if let Some(content) = &self.content {
            content.set(vrr);
        }
    }

    pub fn request(&self, tag: u64) {
        let _ = tag;
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd"
        ))]
        if let Some(feedback) = &self.feedback {
            feedback.request(tag);
        }
    }

    pub fn shown(&mut self) -> Vec<Shown> {
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd"
        ))]
        if let Some(feedback) = &mut self.feedback {
            return feedback.take();
        }
        Vec::new()
    }
}
