use pfx_gpu::window::{Size, Viewport};
use pfx_input::InputEvent;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Warp {
    pub(crate) layout: [f32; 2],
    pub(crate) window: [f32; 2],
}

impl Warp {
    pub(crate) fn events(self) -> [InputEvent; 2] {
        [
            InputEvent::PointerLeft,
            InputEvent::PointerAt {
                layout: self.layout,
                inside: true,
            },
        ]
    }
}

pub(crate) fn warp(point: [f32; 2], viewport: Option<Viewport>, window: Size) -> Warp {
    let edge = |length: u32| length.max(1) as f32 - 1.0;
    let Some(viewport) = viewport else {
        let at = [
            point[0].clamp(0.0, edge(window.width)),
            point[1].clamp(0.0, edge(window.height)),
        ];
        return Warp {
            layout: at,
            window: at,
        };
    };
    let letterbox = viewport.letterbox;
    let visible = letterbox.visible;
    let [x, y] = letterbox.map(point[0], point[1]);
    let render = [
        x.clamp(visible.x, visible.x + (visible.width - 1.0).max(0.0)),
        y.clamp(visible.y, visible.y + (visible.height - 1.0).max(0.0)),
    ];
    let layout = if render == [x, y] {
        point
    } else {
        letterbox.unmap(render[0], render[1])
    };
    let scale = |value: f32, to: u32, from: u32| value * to as f32 / from.max(1) as f32;
    Warp {
        layout,
        window: [
            scale(render[0], viewport.window.width, viewport.render.width)
                .clamp(0.0, edge(viewport.window.width)),
            scale(render[1], viewport.window.height, viewport.render.height)
                .clamp(0.0, edge(viewport.window.height)),
        ],
    }
}

#[cfg(test)]
mod tests {
    use pfx_gpu::window::{LetterboxRounding, RenderScale, viewport_with};
    use pfx_input::{Actions, Button, DeviceKind, Input, PadId};

    use super::*;

    fn letterboxed() -> Viewport {
        viewport_with(
            Size {
                width: 1920,
                height: 1200,
            },
            RenderScale::Native,
            Size {
                width: 1280,
                height: 720,
            },
            LetterboxRounding::Nearest,
        )
        .unwrap()
    }

    #[test]
    fn a_warp_lands_where_the_pointer_reads_it_back() {
        let viewport = letterboxed();
        let window = viewport.window;
        for point in [[0.0, 0.0], [640.0, 360.0], [1279.0, 719.0], [33.5, 701.25]] {
            let warp = warp(point, Some(viewport), window);
            let back = viewport.pointer_to_layout(warp.window[0], warp.window[1]);
            assert!(viewport.pointer_inside(warp.window[0], warp.window[1]));
            assert!((back[0] - point[0]).abs() < 1e-3, "{point:?} -> {back:?}");
            assert!((back[1] - point[1]).abs() < 1e-3, "{point:?} -> {back:?}");
            assert_eq!(warp.layout, point);
        }
    }

    #[test]
    fn a_warp_outside_the_view_is_clamped_into_it() {
        let viewport = letterboxed();
        let warp = warp([-50.0, 9000.0], Some(viewport), viewport.window);
        assert!(viewport.pointer_inside(warp.window[0], warp.window[1]));
        assert!(warp.layout[0] >= 0.0 && warp.layout[0] < 1.0);
        assert!(warp.layout[1] > 718.0 && warp.layout[1] <= 720.0);
        let bare = super::warp(
            [-5.0, 900.0],
            None,
            Size {
                width: 640,
                height: 480,
            },
        );
        assert_eq!(bare.window, [0.0, 479.0]);
        assert_eq!(bare.layout, bare.window);
    }

    #[test]
    fn a_warp_is_not_read_as_the_player_taking_the_mouse() {
        let mut input = Input::new(Actions::new(Vec::new()).unwrap(), 16_667);
        input.feed(InputEvent::PointerAt {
            layout: [100.0, 100.0],
            inside: true,
        });
        input.feed(InputEvent::PadButton {
            pad: PadId(0),
            button: Button::South,
            value: 1.0,
        });
        input.step();
        assert_eq!(input.active().kind, DeviceKind::Gamepad);
        let warp = warp([900.0, 600.0], Some(letterboxed()), letterboxed().window);
        for event in warp.events() {
            input.feed(event);
        }
        let step = input.step();
        assert_eq!(step.changed, None);
        assert_eq!(input.active().kind, DeviceKind::Gamepad);
        assert_eq!(input.pointer().layout, Some([900.0, 600.0]));
        assert!(input.pointer().inside);
    }

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
