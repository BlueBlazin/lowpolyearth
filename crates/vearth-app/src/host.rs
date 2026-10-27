//! The application window as seen by rendering and UI code.
//!
//! Natively this wraps a winit window. In the browser the app renders into an
//! `OffscreenCanvas` owned by the root worker; the page's main thread sends
//! resize and input events in, and this type posts page-level requests
//! (cursor visibility, title) back out.

use crate::input::PhysicalSize;

/// Natively the app renders below the display's resolution when the render
/// resolution setting caps the pixels per point: sizes, scale factor and
/// cursor positions are reported in render pixels, and the system scales the
/// drawable up to the window, as a browser does with a smaller canvas. egui
/// receives winit's events unchanged and converts them to points itself.
#[cfg(not(target_arch = "wasm32"))]
pub struct Window {
    inner: std::sync::Arc<winit::window::Window>,
    /// `f64` bits of the largest render pixels per point; 0 is uncapped.
    max_pixel_ratio: std::sync::atomic::AtomicU64,
}

#[cfg(not(target_arch = "wasm32"))]
impl Window {
    pub fn new(inner: std::sync::Arc<winit::window::Window>) -> Self {
        Self {
            inner,
            max_pixel_ratio: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Caps the pixels per point the app renders at; true when that changed
    /// the render size, which the caller then applies.
    pub fn set_max_pixel_ratio(&self, cap: Option<f64>) -> bool {
        let before = self.render_scale();
        self.max_pixel_ratio.store(
            cap.unwrap_or(0.0).to_bits(),
            std::sync::atomic::Ordering::Relaxed,
        );
        self.render_scale() != before
    }

    /// Render pixels per window pixel, at most 1.
    fn render_scale(&self) -> f64 {
        let cap = f64::from_bits(
            self.max_pixel_ratio
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        if cap > 0.0 {
            (cap / self.inner.scale_factor()).min(1.0)
        } else {
            1.0
        }
    }

    /// `event` with window pixels converted to render pixels.
    pub fn to_render(&self, event: crate::input::WindowEvent) -> crate::input::WindowEvent {
        use crate::input::{MouseScrollDelta, PhysicalPosition, WindowEvent};
        let scale = self.render_scale();
        let position = |p: PhysicalPosition<f64>| PhysicalPosition {
            x: p.x * scale,
            y: p.y * scale,
        };
        match event {
            WindowEvent::Resized(size) => WindowEvent::Resized(self.scaled(size, scale)),
            WindowEvent::ScaleFactorChanged { .. } => WindowEvent::ScaleFactorChanged {
                scale_factor: self.scale_factor(),
            },
            WindowEvent::CursorMoved { position: p } => WindowEvent::CursorMoved {
                position: position(p),
            },
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::PixelDelta(p),
            } => WindowEvent::MouseWheel {
                delta: MouseScrollDelta::PixelDelta(position(p)),
            },
            event => event,
        }
    }

    fn scaled(&self, size: PhysicalSize<u32>, scale: f64) -> PhysicalSize<u32> {
        let side = |pixels: u32| (f64::from(pixels) * scale).round() as u32;
        PhysicalSize::new(side(size.width), side(size.height))
    }

    pub fn winit(&self) -> &winit::window::Window {
        &self.inner
    }

    pub fn winit_arc(&self) -> std::sync::Arc<winit::window::Window> {
        self.inner.clone()
    }

    pub fn request_redraw(&self) {
        self.inner.request_redraw();
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        self.inner.set_cursor_visible(visible);
    }

    pub fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }

    /// The render size in pixels.
    pub fn inner_size(&self) -> PhysicalSize<u32> {
        let size = self.inner.inner_size();
        self.scaled(
            PhysicalSize::new(size.width, size.height),
            self.render_scale(),
        )
    }

    /// Render pixels per point.
    pub fn scale_factor(&self) -> f64 {
        self.inner.scale_factor() * self.render_scale()
    }

    pub fn focus_window(&self) {
        self.inner.focus_window();
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::Window;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::PhysicalSize;
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::JsCast;

    /// The canvas the app renders into. Lives on the root worker only.
    pub struct Window {
        canvas: web_sys::OffscreenCanvas,
        size: Cell<PhysicalSize<u32>>,
        scale_factor: Cell<f64>,
        redraw_requested: Cell<bool>,
        cursor_visible: Cell<Option<bool>>,
        title: RefCell<String>,
    }

    impl Window {
        pub fn new(canvas: web_sys::OffscreenCanvas, size: PhysicalSize<u32>, scale: f64) -> Self {
            Self {
                canvas,
                size: Cell::new(size),
                scale_factor: Cell::new(scale),
                redraw_requested: Cell::new(true),
                cursor_visible: Cell::new(None),
                title: RefCell::new(String::new()),
            }
        }

        pub fn canvas(&self) -> &web_sys::OffscreenCanvas {
            &self.canvas
        }

        /// Records a new size from the page; the next surface configure
        /// resizes the canvas backing store.
        pub fn set_size(&self, size: PhysicalSize<u32>, scale_factor: f64) {
            self.size.set(size);
            self.scale_factor.set(scale_factor);
        }

        /// Returns and clears the pending redraw request.
        pub fn take_redraw_request(&self) -> bool {
            self.redraw_requested.replace(false)
        }

        pub fn request_redraw(&self) {
            self.redraw_requested.set(true);
        }

        pub fn set_cursor_visible(&self, visible: bool) {
            if self.cursor_visible.replace(Some(visible)) != Some(visible) {
                post_to_page("cursorVisible", visible.into());
            }
        }

        pub fn set_title(&self, title: &str) {
            if *self.title.borrow() != title {
                *self.title.borrow_mut() = title.to_owned();
                post_to_page("title", title.into());
            }
        }

        pub fn inner_size(&self) -> PhysicalSize<u32> {
            self.size.get()
        }

        pub fn scale_factor(&self) -> f64 {
            self.scale_factor.get()
        }

        pub fn focus_window(&self) {}
    }

    /// Sends a `{ type, value }` request to the page's main thread.
    pub fn post_to_page(kind: &str, value: wasm_bindgen::JsValue) {
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&message, &"type".into(), &kind.into());
        let _ = js_sys::Reflect::set(&message, &"value".into(), &value);
        let scope = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
        if let Err(error) = scope.post_message(&message) {
            log::warn!("could not post {kind} to the page: {error:?}");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::post_to_page;

/// Whether the host reports under 8 GB of memory. Native builds target Macs
/// that meet the app's requirements; browsers report a coarse
/// `navigator.deviceMemory`.
pub fn low_memory() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    return false;
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        static LOW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *LOW.get_or_init(|| {
            let navigator = js_sys::global()
                .unchecked_into::<web_sys::WorkerGlobalScope>()
                .navigator();
            js_sys::Reflect::get(&navigator, &"deviceMemory".into())
                .ok()
                .and_then(|memory| memory.as_f64())
                .is_some_and(|gigabytes| gigabytes < 8.0)
        })
    }
}

/// Whether the platform's shortcut modifier is Command (macOS) rather than
/// Ctrl, for labels; input handling accepts either.
pub fn command_modifier_is_cmd() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    return cfg!(target_os = "macos");
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        static MAC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *MAC.get_or_init(|| {
            js_sys::global()
                .unchecked_into::<web_sys::WorkerGlobalScope>()
                .navigator()
                .user_agent()
                .is_ok_and(|agent| agent.contains("Mac"))
        })
    }
}
