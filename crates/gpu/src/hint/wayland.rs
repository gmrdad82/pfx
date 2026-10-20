use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    Connection, Dispatch as Handler, EventQueue, Proxy, QueueHandle, delegate_noop,
};
use wayland_protocols::wp::content_type::v1::client::wp_content_type_manager_v1::WpContentTypeManagerV1;
use wayland_protocols::wp::content_type::v1::client::wp_content_type_v1::{Type, WpContentTypeV1};
use wayland_protocols::wp::presentation_time::client::wp_presentation::WpPresentation;
use wayland_protocols::wp::presentation_time::client::wp_presentation_feedback::{
    self, WpPresentationFeedback,
};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::Window;

use crate::presents::Shown;

struct Quiet;

impl Handler<WlRegistry, GlobalListContents> for Quiet {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Quiet: WpContentTypeManagerV1);
delegate_noop!(Quiet: WpContentTypeV1);

pub struct ContentHint {
    connection: Connection,
    hint: WpContentTypeV1,
    manager: WpContentTypeManagerV1,
    _queue: EventQueue<Quiet>,
}

impl ContentHint {
    pub fn new(window: &Window) -> Option<Self> {
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
            return None;
        };
        let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<Quiet>(&connection).ok()?;
        let handle = queue.handle();
        let manager: WpContentTypeManagerV1 = globals.bind(&handle, 1..=1, ()).ok()?;
        let id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }
                .ok()?;
        let surface = WlSurface::from_id(&connection, id).ok()?;
        let hint = manager.get_surface_content_type(&surface, &handle, ());
        connection.flush().ok()?;
        Some(Self {
            connection,
            hint,
            manager,
            _queue: queue,
        })
    }

    pub fn set(&self, vrr: bool) {
        self.hint
            .set_content_type(if vrr { Type::Game } else { Type::None });
        let _ = self.connection.flush();
    }
}

impl Drop for ContentHint {
    fn drop(&mut self) {
        self.hint.destroy();
        self.manager.destroy();
        let _ = self.connection.flush();
    }
}

#[derive(Default)]
struct Feedbacks {
    shown: Vec<Shown>,
}

impl Handler<WlRegistry, GlobalListContents> for Feedbacks {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Feedbacks: ignore WpPresentation);

impl Handler<WpPresentationFeedback, u64> for Feedbacks {
    fn event(
        state: &mut Self,
        _: &WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        tag: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                ..
            } => {
                let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                state.shown.push(Shown {
                    tag: *tag,
                    at_ns: Some(
                        seconds
                            .saturating_mul(1_000_000_000)
                            .saturating_add(u64::from(tv_nsec)),
                    ),
                    refresh_ns: Some(u64::from(refresh)),
                });
            }
            wp_presentation_feedback::Event::Discarded => state.shown.push(Shown {
                tag: *tag,
                at_ns: None,
                refresh_ns: None,
            }),
            _ => {}
        }
    }
}

pub struct PresentationFeedback {
    connection: Connection,
    surface: WlSurface,
    presentation: WpPresentation,
    queue: EventQueue<Feedbacks>,
    state: Feedbacks,
}

impl PresentationFeedback {
    pub fn new(window: &Window) -> Option<Self> {
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
            return None;
        };
        let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<Feedbacks>(&connection).ok()?;
        let presentation: WpPresentation = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        let id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }
                .ok()?;
        let surface = WlSurface::from_id(&connection, id).ok()?;
        Some(Self {
            connection,
            surface,
            presentation,
            queue,
            state: Feedbacks::default(),
        })
    }

    pub fn request(&self, tag: u64) {
        self.presentation
            .feedback(&self.surface, &self.queue.handle(), tag);
        let _ = self.connection.flush();
    }

    pub fn take(&mut self) -> Vec<Shown> {
        let _ = self.queue.dispatch_pending(&mut self.state);
        std::mem::take(&mut self.state.shown)
    }
}

impl Drop for PresentationFeedback {
    fn drop(&mut self) {
        self.presentation.destroy();
        let _ = self.connection.flush();
    }
}
