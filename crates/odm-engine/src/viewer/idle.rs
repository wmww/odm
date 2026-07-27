//! Hosting the event loop: `run_viewer` plus the winit shim that keeps an
//! unseen window from busy-looping.

use super::ViewerApp;
use crate::state::EngineState;
use eframe::egui;
use std::sync::Arc;

pub fn run_viewer(state: Arc<EngineState>) -> Result<(), String> {
    let title = format!(
        "ODM — {}",
        state.project().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    );
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 840.0]).with_title(&title),
        ..Default::default()
    };
    // Own the event loop rather than `eframe::run_native`, so `SlowIdle` can see
    // and fix up the control flow eframe leaves behind.
    let event_loop = winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let mut app = SlowIdle {
        inner: eframe::create_native(
            "odm",
            options,
            Box::new(move |cc| Ok(Box::new(ViewerApp::new(cc, state)))),
            &event_loop,
        ),
        clamped_until: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}

/// Backstop against a busy-looping event loop while the window is not visible.
///
/// When a repaint falls due, eframe asks winit to redraw and parks the loop in
/// `ControlFlow::Poll`, expecting `RedrawRequested` right back. A Wayland
/// surface that nobody is displaying (occluded, other workspace, output off)
/// never gets its frame callback, so winit withholds that event — and `Poll`
/// spins a core until the window is shown again. eframe only guards the
/// Windows/macOS form of this, via `Window::is_visible`, which Wayland does not
/// answer.
///
/// So: whenever eframe leaves `Poll` set, downgrade it to a timer. Events still
/// wake the loop immediately, so a visible window is unaffected — it paints and
/// goes back to `Wait` before we ever look.
struct SlowIdle<'a> {
    inner: eframe::EframeWinitApplication<'a>,
    /// Deadline we installed, to recognize (and re-arm) our own expired timer.
    clamped_until: Option<std::time::Instant>,
}

/// How often a loop stuck in `Poll` wakes up to check for work. Only ever hit
/// while nothing is displaying the window, so it costs nothing to keep short.
const IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(100);

impl SlowIdle<'_> {
    fn clamp(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::event_loop::ControlFlow;
        let now = std::time::Instant::now();
        // Re-arm our own timer too: eframe leaves an expired `WaitUntil` in
        // place when it wakes up with nothing to do, which winit treats as
        // "wake immediately" — the same spin by another name.
        let stuck = match event_loop.control_flow() {
            ControlFlow::Poll => true,
            ControlFlow::WaitUntil(t) => self.clamped_until == Some(t) && t <= now,
            ControlFlow::Wait => false,
        };
        if stuck {
            let until = now + IDLE_POLL;
            self.clamped_until = Some(until);
            event_loop.set_control_flow(ControlFlow::WaitUntil(until));
        }
    }
}

impl winit::application::ApplicationHandler<eframe::UserEvent> for SlowIdle<'_> {
    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
        self.clamp(event_loop);
    }

    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn new_events(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        self.inner.new_events(event_loop, cause);
    }

    fn user_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        event: eframe::UserEvent,
    ) {
        self.inner.user_event(event_loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        self.inner.window_event(event_loop, window_id, event);
    }

    fn device_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        self.inner.device_event(event_loop, device_id, event);
    }

    fn exiting(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.memory_warning(event_loop);
    }
}
