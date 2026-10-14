use pfx_gpu::window::Size;
use pfx_play::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pointer {
    applied: Cursor,
    focused: bool,
    minimized: bool,
}

impl Pointer {
    pub(crate) fn new(focused: bool) -> Self {
        Self {
            applied: Cursor::Shown,
            focused,
            minimized: false,
        }
    }

    pub(crate) fn focus(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub(crate) fn minimize(&mut self, minimized: bool) {
        self.minimized = minimized;
    }

    pub(crate) fn change(&mut self, asked: Cursor) -> Option<Cursor> {
        let wanted = if self.focused && !self.minimized {
            asked
        } else {
            Cursor::Shown
        };
        (wanted != self.applied).then(|| {
            self.applied = wanted;
            wanted
        })
    }
}

pub(crate) fn nudge(at: [f32; 2], delta: [f64; 2], size: Size) -> [f32; 2] {
    let bound = |value: f64, length: u32| value.clamp(0.0, f64::from(length.max(1) - 1)) as f32;
    [
        bound(f64::from(at[0]) + delta[0], size.width),
        bound(f64::from(at[1]) + delta[1], size.height),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_captured_pointer_is_given_back_while_the_window_is_away() {
        let mut pointer = Pointer::new(true);
        assert_eq!(pointer.change(Cursor::Captured), Some(Cursor::Captured));
        assert_eq!(pointer.change(Cursor::Captured), None);
        pointer.focus(false);
        assert_eq!(pointer.change(Cursor::Captured), Some(Cursor::Shown));
        pointer.focus(true);
        pointer.minimize(true);
        assert_eq!(pointer.change(Cursor::Captured), None);
        pointer.minimize(false);
        assert_eq!(pointer.change(Cursor::Captured), Some(Cursor::Captured));
        assert_eq!(pointer.change(Cursor::Shown), Some(Cursor::Shown));
    }
}
