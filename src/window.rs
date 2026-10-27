//! Native window backed by winit (optional `window` feature).
//!
//! [`WinitShell`] implements [`EventPump`] on top of winit's `pump_app_events`, so the
//! engine keeps ownership of the loop and of frame pacing (sleep-based, no busy-wait). Winit
//! creates the window during the first pump; frames start once it exists.
//!
//! Known limits of this model, from winit's documentation for `pump_app_events`:
//! - available on Windows, macOS, Linux (X11 and Wayland) and Android only, not on web or iOS;
//! - winit recommends rendering inside its own callbacks. A future renderer may need to draw
//!   from `WindowEvent::RedrawRequested`, and macOS may show resize artifacts otherwise;
//! - while the OS runs a modal loop (for example dragging a window on Windows), frames can
//!   stall.
//!
//! Only the close request is handled for now; input is not forwarded to the application.
//! This code has no automated test (it needs a display); the driver logic around it is
//! covered by the scripted tests in [`crate::shell`].

use std::error::Error;
use std::fmt;
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::error::{EventLoopError, OsError};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Window, WindowId};

use crate::app::{Application, Engine, RunReport};
use crate::clock::Clock;
use crate::shell::{EventPump, ShellError, ShellState, run_with_shell};

/// Initial window settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Initial inner width in physical pixels.
    pub width: u32,
    /// Initial inner height in physical pixels.
    pub height: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: String::from("Apairon"),
            width: 1280,
            height: 720,
        }
    }
}

/// Window or event-loop failure.
#[derive(Debug)]
pub enum WindowError {
    /// The OS event loop could not be created.
    EventLoop(EventLoopError),
    /// The window could not be created.
    CreateWindow(OsError),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(e) => write!(f, "cannot create the event loop: {e}"),
            Self::CreateWindow(e) => write!(f, "cannot create the window: {e}"),
        }
    }
}

impl Error for WindowError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EventLoop(e) => Some(e),
            Self::CreateWindow(e) => Some(e),
        }
    }
}

struct Handler {
    config: WindowConfig,
    window: Option<Window>,
    close_requested: bool,
    creation_error: Option<OsError>,
}

impl ApplicationHandler for Handler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(PhysicalSize::new(self.config.width, self.config.height));
        match event_loop.create_window(attributes) {
            Ok(window) => self.window = Some(window),
            Err(error) => self.creation_error = Some(error),
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let ours = self
            .window
            .as_ref()
            .is_some_and(|window| window.id() == window_id);
        if ours && matches!(event, WindowEvent::CloseRequested) {
            self.close_requested = true;
        }
    }
}

/// A window and its OS event loop.
pub struct WinitShell {
    // Declared before `event_loop` so the window is dropped first.
    handler: Handler,
    event_loop: EventLoop<()>,
}

impl WinitShell {
    /// Creates the event loop. The window itself is created during the first pump.
    pub fn new(config: WindowConfig) -> Result<Self, WindowError> {
        let event_loop = EventLoop::new().map_err(WindowError::EventLoop)?;
        Ok(Self {
            handler: Handler {
                config,
                window: None,
                close_requested: false,
                creation_error: None,
            },
            event_loop,
        })
    }
}

impl EventPump for WinitShell {
    type Error = WindowError;

    fn pump(&mut self, timeout: Duration) -> Result<ShellState, WindowError> {
        let status = self
            .event_loop
            .pump_app_events(Some(timeout), &mut self.handler);

        if let Some(error) = self.handler.creation_error.take() {
            return Err(WindowError::CreateWindow(error));
        }
        if self.handler.close_requested || matches!(status, PumpStatus::Exit(_)) {
            return Ok(ShellState::Closed);
        }
        if self.handler.window.is_some() {
            Ok(ShellState::Ready)
        } else {
            Ok(ShellState::Waiting)
        }
    }
}

/// Opens a window and runs `app` in it until the window is closed or the application exits.
pub fn run_windowed<A, C>(
    engine: Engine,
    app: &mut A,
    clock: &C,
    config: WindowConfig,
) -> Result<RunReport, ShellError<WindowError, A::Error>>
where
    A: Application,
    C: Clock,
{
    let mut shell = WinitShell::new(config).map_err(ShellError::Pump)?;
    run_with_shell(engine, app, clock, &mut shell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_window_config_is_valid() {
        let config = WindowConfig::default();
        assert_eq!(config.title, "Apairon");
        assert!(config.width > 0 && config.height > 0);
    }
}
