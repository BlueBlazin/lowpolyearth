//! egui input and platform output for each host: egui-winit natively, and
//! events forwarded from the page's main thread in the browser.

use crate::host::Window;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type UiEvent = winit::event::WindowEvent;

#[cfg(target_arch = "wasm32")]
pub(crate) use web::{EguiInput, UiEvent};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct EguiInput(egui_winit::State);

#[cfg(not(target_arch = "wasm32"))]
impl EguiInput {
    pub fn new(ctx: egui::Context, window: &Window) -> Self {
        Self(egui_winit::State::new(
            ctx,
            egui::ViewportId::ROOT,
            window.winit(),
            Some(window.scale_factor() as f32),
            None,
            None,
        ))
    }

    /// Feeds an event to egui; true when egui consumed it.
    pub fn on_event(&mut self, window: &Window, event: &UiEvent) -> bool {
        self.0.on_window_event(window.winit(), event).consumed
    }

    pub fn take_egui_input(&mut self, window: &Window) -> egui::RawInput {
        let mut input = self.0.take_egui_input(window.winit());
        // egui-winit converts events to points with winit's own scale factor;
        // the UI is drawn at the render scale (see `Window`).
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(window.scale_factor() as f32);
        input
    }

    pub fn handle_platform_output(&mut self, window: &Window, output: egui::PlatformOutput) {
        self.0.handle_platform_output(window.winit(), output);
    }

    pub fn clipboard_text(&mut self) -> Option<String> {
        self.0.clipboard_text()
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::Window;
    use crate::input::{ElementState, WindowEvent};
    use vearth_platform::time::Instant;

    /// One page event: the application's view of it, plus the egui events it
    /// produces (text, clipboard and named keys need the page's `key` value).
    pub(crate) struct UiEvent {
        pub window: Option<WindowEvent>,
        pub egui: Vec<egui::Event>,
    }

    pub(crate) struct EguiInput {
        ctx: egui::Context,
        pending: egui::RawInput,
        started: Instant,
        pointer_position: Option<egui::Pos2>,
        cursor: Option<egui::CursorIcon>,
        ime_cursor: Option<egui::Rect>,
    }

    impl EguiInput {
        pub fn new(ctx: egui::Context, _window: &Window) -> Self {
            Self {
                ctx,
                pending: egui::RawInput::default(),
                started: Instant::now(),
                pointer_position: None,
                cursor: None,
                ime_cursor: None,
            }
        }

        /// Feeds an event to egui; true when egui consumed it, matching
        /// egui-winit's rules.
        pub fn on_event(&mut self, window: &Window, event: &UiEvent) -> bool {
            let points = window.scale_factor() as f32;
            let mut consumed = false;
            if let Some(event) = &event.window {
                consumed |= self.on_window_event(event, points);
            }
            if !event.egui.is_empty() {
                self.pending.events.extend(event.egui.iter().cloned());
                // Tab moves egui focus, so it always belongs to egui.
                let tab = event.egui.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Tab,
                            ..
                        }
                    )
                });
                consumed |= self.ctx.egui_wants_keyboard_input() || tab;
            }
            consumed
        }

        fn on_window_event(&mut self, event: &WindowEvent, points: f32) -> bool {
            match event {
                WindowEvent::CursorMoved { position } => {
                    let pos = egui::pos2(position.x as f32 / points, position.y as f32 / points);
                    self.pointer_position = Some(pos);
                    self.pending.events.push(egui::Event::PointerMoved(pos));
                    self.ctx.egui_is_using_pointer()
                }
                WindowEvent::CursorLeft => {
                    self.pointer_position = None;
                    self.pending.events.push(egui::Event::PointerGone);
                    false
                }
                WindowEvent::MouseInput { state, button } => {
                    let button = match button {
                        crate::input::MouseButton::Left => egui::PointerButton::Primary,
                        crate::input::MouseButton::Right => egui::PointerButton::Secondary,
                        crate::input::MouseButton::Middle => egui::PointerButton::Middle,
                        crate::input::MouseButton::Back => egui::PointerButton::Extra1,
                        crate::input::MouseButton::Forward => egui::PointerButton::Extra2,
                        crate::input::MouseButton::Other(_) => return false,
                    };
                    if let Some(pos) = self.pointer_position {
                        self.pending.events.push(egui::Event::PointerButton {
                            pos,
                            button,
                            pressed: *state == ElementState::Pressed,
                            modifiers: self.pending.modifiers,
                        });
                    }
                    self.ctx.egui_wants_pointer_input()
                }
                WindowEvent::MouseWheel { delta } => {
                    let (unit, delta) = match delta {
                        crate::input::MouseScrollDelta::LineDelta(x, y) => {
                            (egui::MouseWheelUnit::Line, egui::vec2(*x, *y))
                        }
                        crate::input::MouseScrollDelta::PixelDelta(p) => (
                            egui::MouseWheelUnit::Point,
                            egui::vec2(p.x as f32 / points, p.y as f32 / points),
                        ),
                    };
                    self.pending.events.push(egui::Event::MouseWheel {
                        unit,
                        delta,
                        phase: egui::TouchPhase::Move,
                        modifiers: self.pending.modifiers,
                    });
                    self.ctx.egui_wants_pointer_input()
                }
                WindowEvent::ModifiersChanged(m) => {
                    self.pending.modifiers = egui::Modifiers {
                        alt: m.alt,
                        ctrl: m.control,
                        shift: m.shift,
                        mac_cmd: false,
                        command: m.control || m.logo,
                    };
                    false
                }
                WindowEvent::Focused(focused) => {
                    self.pending.focused = *focused;
                    self.pending
                        .events
                        .push(egui::Event::WindowFocused(*focused));
                    false
                }
                WindowEvent::KeyboardInput { .. } => self.ctx.egui_wants_keyboard_input(),
                _ => false,
            }
        }

        pub fn take_egui_input(&mut self, window: &Window) -> egui::RawInput {
            let points = window.scale_factor() as f32;
            let size = window.inner_size();
            let mut input = std::mem::take(&mut self.pending);
            self.pending.modifiers = input.modifiers;
            self.pending.focused = input.focused;
            input.time = Some(self.started.elapsed().as_secs_f64());
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.width as f32 / points, size.height as f32 / points),
            ));
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(points);
            input
        }

        pub fn handle_platform_output(&mut self, _window: &Window, output: egui::PlatformOutput) {
            if self.cursor.replace(output.cursor_icon) != Some(output.cursor_icon) {
                crate::host::post_to_page("cursorIcon", cursor_css(output.cursor_icon).into());
            }
            // The page places its text agent at the caret so input-method
            // candidate windows appear beside the text being edited.
            let ime_cursor = output.ime.map(|ime| ime.cursor_rect);
            if self.ime_cursor != ime_cursor {
                self.ime_cursor = ime_cursor;
                let value = match ime_cursor {
                    Some(rect) => js_sys::Array::of3(
                        &rect.min.x.into(),
                        &rect.min.y.into(),
                        &rect.height().into(),
                    )
                    .into(),
                    None => wasm_bindgen::JsValue::NULL,
                };
                crate::host::post_to_page("imeCursor", value);
            }
            for command in output.commands {
                match command {
                    egui::OutputCommand::CopyText(text) => {
                        crate::host::post_to_page("copyText", text.into())
                    }
                    egui::OutputCommand::OpenUrl(open) => {
                        crate::host::post_to_page("openUrl", open.url.into())
                    }
                    _ => {}
                }
            }
        }

        /// Pasted text arrives as `egui::Event::Paste` from the page.
        pub fn clipboard_text(&mut self) -> Option<String> {
            None
        }
    }

    fn cursor_css(icon: egui::CursorIcon) -> &'static str {
        use egui::CursorIcon as C;
        match icon {
            C::Default => "default",
            C::None => "none",
            C::ContextMenu => "context-menu",
            C::Help => "help",
            C::PointingHand => "pointer",
            C::Progress => "progress",
            C::Wait => "wait",
            C::Cell => "cell",
            C::Crosshair => "crosshair",
            C::Text => "text",
            C::VerticalText => "vertical-text",
            C::Alias => "alias",
            C::Copy => "copy",
            C::Move => "move",
            C::NoDrop => "no-drop",
            C::NotAllowed => "not-allowed",
            C::Grab => "grab",
            C::Grabbing => "grabbing",
            C::AllScroll => "all-scroll",
            C::ResizeHorizontal | C::ResizeColumn => "ew-resize",
            C::ResizeNeSw => "nesw-resize",
            C::ResizeNwSe => "nwse-resize",
            C::ResizeVertical | C::ResizeRow => "ns-resize",
            C::ResizeEast => "e-resize",
            C::ResizeSouthEast => "se-resize",
            C::ResizeSouth => "s-resize",
            C::ResizeSouthWest => "sw-resize",
            C::ResizeWest => "w-resize",
            C::ResizeNorthWest => "nw-resize",
            C::ResizeNorth => "n-resize",
            C::ResizeNorthEast => "ne-resize",
            C::ZoomIn => "zoom-in",
            C::ZoomOut => "zoom-out",
        }
    }
}
