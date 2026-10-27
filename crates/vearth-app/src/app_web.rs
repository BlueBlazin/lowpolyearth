//! Browser host for [`App`]: the root worker's frame loop and the entry
//! points for input forwarded from the page.
//!
//! Frames follow the worker's `requestAnimationFrame`, which the browser
//! aligns to display refresh and pauses for hidden pages; while the page is
//! hidden a slow timer keeps loading and timeouts moving, like the native
//! host's occluded-window cadence.

use super::{App, PhotoStatus, StartPos, State};
use crate::capture::{CaptureFrame, PendingCapture, PhotoArtifact, PhotoRecipe, PhotoSettings};
use crate::egui_input::UiEvent;
use crate::gfx::Gfx;
use crate::host::{Window, post_to_page};
use crate::input::{PhysicalSize, WindowEvent};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use vearth_platform::time::Instant;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

pub(super) enum CaptureTarget {
    Photo {
        settings: PhotoSettings,
        recipe: PhotoRecipe,
    },
    Screenshot,
}

pub(super) struct PendingWebCapture {
    capture: PendingCapture,
    target: CaptureTarget,
    width: u32,
    height: u32,
}

impl State {
    /// Starts a capture whose pixels arrive on a later frame.
    pub(super) fn submit_web_capture(&mut self, frame: CaptureFrame<'_>, target: CaptureTarget) {
        let (width, height) = (frame.width, frame.height);
        match crate::capture::submit_capture(&mut self.gfx.renderer, frame) {
            Ok(capture) => {
                self.pending_capture = Some(PendingWebCapture {
                    capture,
                    target,
                    width,
                    height,
                })
            }
            Err(error) => self.capture_failed(&target, error.to_string()),
        }
    }

    /// Delivers a finished capture to the page as a download.
    pub(super) fn poll_web_capture(&mut self) {
        let Some(pending) = self.pending_capture.take() else {
            return;
        };
        let pixels = match pending.capture.try_finish() {
            Err(capture) => {
                self.pending_capture = Some(PendingWebCapture { capture, ..pending });
                return;
            }
            Ok(Ok(pixels)) => pixels,
            Ok(Err(error)) => return self.capture_failed(&pending.target, error.to_string()),
        };
        let (width, height) = (pending.width, pending.height);
        let encoded = match &pending.target {
            CaptureTarget::Photo { settings, recipe } => {
                crate::capture::photo_download(settings, width, height, &pixels.rgba, *recipe)
            }
            CaptureTarget::Screenshot => encode_png(width, height, &pixels.rgba).map(|png| {
                let seconds = vearth_platform::time::SystemTime::now()
                    .duration_since(vearth_platform::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                (format!("low-poly-earth-screenshot-{seconds}.png"), png)
            }),
        };
        let (name, png) = match encoded {
            Ok(encoded) => encoded,
            Err(error) => return self.capture_failed(&pending.target, error.to_string()),
        };
        download(&name, "image/png", &png);
        log::info!("capture: offered {name} ({} KiB)", png.len() / 1024);
        match pending.target {
            CaptureTarget::Photo { .. } => {
                self.photo_status = PhotoStatus::Saved(PhotoArtifact {
                    image_path: name.clone().into(),
                    recipe_path: name.into(),
                    width,
                    height,
                });
            }
            CaptureTarget::Screenshot => self.ui.set_screenshot_notice(format!(
                "Downloaded {name}\nClick to take another screenshot"
            )),
        }
    }

    fn capture_failed(&mut self, target: &CaptureTarget, error: String) {
        log::error!("capture failed: {error}");
        match target {
            CaptureTarget::Photo { .. } => self.photo_status = PhotoStatus::Failed(error),
            CaptureTarget::Screenshot => self
                .ui
                .set_screenshot_notice(format!("Screenshot failed: {error}\nClick to retry")),
        }
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(png)
}

/// Asks the page to save `bytes` as a file.
fn download(name: &str, mime: &str, bytes: &[u8]) {
    let value = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&value, &"name".into(), &name.into());
    let _ = js_sys::Reflect::set(&value, &"mime".into(), &mime.into());
    let _ = js_sys::Reflect::set(
        &value,
        &"bytes".into(),
        &js_sys::Uint8Array::from(bytes).into(),
    );
    post_to_page("download", value.into());
}

impl App {
    /// Offers an egui-only event (paste, text composition) to the UI.
    fn ui_only_event(&mut self, event: &UiEvent) {
        if let Some(state) = self.state.as_mut() {
            state.ui.on_event(&state.window, event);
            state.window.request_redraw();
        }
    }
}

/// Interval between frames while the page is hidden.
const HIDDEN_FRAME_INTERVAL_MS: i32 = 100;

/// The application running in the root worker.
pub struct WebApp {
    app: App,
    window: Arc<Window>,
    hidden: bool,
    frame_scheduled: bool,
    /// Cleared if WebGPU initialization fails, so no frames run.
    running: bool,
}

impl WebApp {
    /// Starts the application on `canvas` (sized in physical pixels) and
    /// begins the frame loop once the GPU device is ready.
    pub fn start(
        canvas: web_sys::OffscreenCanvas,
        size: PhysicalSize<u32>,
        scale_factor: f64,
        start: StartPos,
    ) -> Rc<RefCell<WebApp>> {
        let window = Arc::new(Window::new(canvas, size, scale_factor));
        let mut app = App::new(start);
        let prepared_world = app.begin_launch(window.clone());
        let web_app = Rc::new(RefCell::new(WebApp {
            app,
            window: window.clone(),
            hidden: false,
            frame_scheduled: false,
            running: true,
        }));
        let launching = web_app.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match Gfx::new(&window).await {
                Ok(gfx) => {
                    launching
                        .borrow_mut()
                        .app
                        .finish_launch(gfx, prepared_world);
                    post_to_page("ready", wasm_bindgen::JsValue::TRUE);
                    schedule_frame(&launching);
                }
                Err(error) => {
                    log::error!("WebGPU initialization failed: {error}");
                    launching.borrow_mut().running = false;
                    post_to_page(
                        "fatal",
                        format!("WebGPU initialization failed: {error}").into(),
                    );
                }
            }
        });
        web_app
    }

    /// Handles one input or window event from the page.
    pub fn event(&mut self, event: Option<WindowEvent>, egui: Vec<egui::Event>) {
        if let Some(WindowEvent::Resized(size)) = &event {
            self.window.set_size(*size, self.window.scale_factor());
        }
        if let Some(WindowEvent::ScaleFactorChanged { scale_factor }) = &event {
            self.window
                .set_size(self.window.inner_size(), *scale_factor);
        }
        let ui_event = UiEvent {
            window: event.clone(),
            egui,
        };
        match event {
            Some(event) => self.app.handle_window_event(event, &ui_event),
            None => self.app.ui_only_event(&ui_event),
        }
    }

    /// The page is being unloaded: stop background work.
    pub fn shutdown(&mut self) {
        self.running = false;
        self.app.shutdown();
    }

    /// The page became hidden or visible.
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
        let occluded = WindowEvent::Occluded(hidden);
        self.app.handle_window_event(
            occluded.clone(),
            &UiEvent {
                window: Some(occluded),
                egui: Vec::new(),
            },
        );
    }

    fn frame(&mut self) {
        self.app.after_events();
        if self.app.exit_requested {
            return;
        }
        if let Some(state) = self.app.state.as_mut() {
            state.poll_web_capture();
        }
        let now = Instant::now();
        let requested = self.window.take_redraw_request();
        if requested || self.app.next_redraw(now) <= now {
            let redraw = WindowEvent::RedrawRequested;
            self.app.handle_window_event(
                redraw.clone(),
                &UiEvent {
                    window: Some(redraw),
                    egui: Vec::new(),
                },
            );
        }
    }
}

/// Keeps one frame callback outstanding: animation frames while visible, a
/// slow timer while hidden.
pub fn schedule_frame(web_app: &Rc<RefCell<WebApp>>) {
    let hidden = {
        let mut app = web_app.borrow_mut();
        if app.frame_scheduled || !app.running {
            return;
        }
        app.frame_scheduled = true;
        app.hidden
    };
    let next = web_app.clone();
    // A panicking frame leaves the app unusable either way; the panic hook
    // reports it.
    let callback = Closure::once_assert_unwind_safe(move || {
        next.borrow_mut().frame_scheduled = false;
        next.borrow_mut().frame();
        if !next.borrow().app.exit_requested {
            schedule_frame(&next);
        }
    })
    .into_js_value();
    let scope = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
    let scheduled = if hidden {
        scope
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.unchecked_ref(),
                HIDDEN_FRAME_INTERVAL_MS,
            )
            .map(|_| ())
    } else {
        scope
            .request_animation_frame(callback.unchecked_ref())
            .map(|_| ())
    };
    if let Err(error) = scheduled {
        log::error!("could not schedule a frame: {error:?}");
    }
}
