//! Window input shared by the native (winit) and browser hosts.
//!
//! Names and shapes follow winit so application code reads the same on both
//! hosts. Physical key names are the W3C `KeyboardEvent.code` values, which
//! winit's `KeyCode` variants also use.

/// A physical position in pixels, origin at the surface's top left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicalPosition<T> {
    pub x: T,
    pub y: T,
}

/// A surface size in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalSize<T> {
    pub width: T,
    pub height: T,
}

impl<T> PhysicalSize<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseScrollDelta {
    /// Lines (or rows) scrolled; positive `y` scrolls content up (zooms in).
    LineDelta(f32, f32),
    /// Pixels scrolled, with the same sign convention.
    PixelDelta(PhysicalPosition<f64>),
}

/// Held modifier keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifiersState {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    /// Command on macOS, the Windows key elsewhere.
    pub logo: bool,
}

impl ModifiersState {
    pub fn shift_key(self) -> bool {
        self.shift
    }

    pub fn control_key(self) -> bool {
        self.control
    }

    pub fn alt_key(self) -> bool {
        self.alt
    }

    pub fn super_key(self) -> bool {
        self.logo
    }
}

macro_rules! key_codes {
    ($($name:ident),* $(,)?) => {
        /// Keys by physical location, named as in `KeyboardEvent.code`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum KeyCode {
            $($name,)*
        }

        impl KeyCode {
            /// Parses a `KeyboardEvent.code` value.
            pub fn from_code(code: &str) -> Option<Self> {
                match code {
                    $(stringify!($name) => Some(Self::$name),)*
                    _ => None,
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            pub fn from_winit(code: winit::keyboard::KeyCode) -> Option<Self> {
                match code {
                    $(winit::keyboard::KeyCode::$name => Some(Self::$name),)*
                    _ => None,
                }
            }
        }
    };
}

key_codes!(
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Enter,
    Escape,
    Space,
    Tab,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
    CapsLock,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhysicalKey {
    Code(KeyCode),
    Unidentified,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub physical_key: PhysicalKey,
    pub state: ElementState,
    pub repeat: bool,
}

/// Window events the application handles.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowEvent {
    Resized(PhysicalSize<u32>),
    ScaleFactorChanged {
        scale_factor: f64,
    },
    Occluded(bool),
    Focused(bool),
    CloseRequested,
    RedrawRequested,
    CursorMoved {
        position: PhysicalPosition<f64>,
    },
    CursorEntered,
    CursorLeft,
    MouseInput {
        state: ElementState,
        button: MouseButton,
    },
    MouseWheel {
        delta: MouseScrollDelta,
    },
    ModifiersChanged(ModifiersState),
    KeyboardInput {
        event: KeyEvent,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl WindowEvent {
    /// The application's view of a winit event, if it handles that kind.
    pub fn from_winit(event: &winit::event::WindowEvent) -> Option<Self> {
        use winit::event::WindowEvent as W;
        let state = |state: winit::event::ElementState| match state {
            winit::event::ElementState::Pressed => ElementState::Pressed,
            winit::event::ElementState::Released => ElementState::Released,
        };
        Some(match event {
            W::Resized(size) => Self::Resized(PhysicalSize::new(size.width, size.height)),
            W::ScaleFactorChanged { scale_factor, .. } => Self::ScaleFactorChanged {
                scale_factor: *scale_factor,
            },
            W::Occluded(occluded) => Self::Occluded(*occluded),
            W::Focused(focused) => Self::Focused(*focused),
            W::CloseRequested => Self::CloseRequested,
            W::RedrawRequested => Self::RedrawRequested,
            W::CursorMoved { position, .. } => Self::CursorMoved {
                position: PhysicalPosition {
                    x: position.x,
                    y: position.y,
                },
            },
            W::CursorEntered { .. } => Self::CursorEntered,
            W::CursorLeft { .. } => Self::CursorLeft,
            W::MouseInput {
                state: pressed,
                button,
                ..
            } => Self::MouseInput {
                state: state(*pressed),
                button: match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    winit::event::MouseButton::Back => MouseButton::Back,
                    winit::event::MouseButton::Forward => MouseButton::Forward,
                    winit::event::MouseButton::Other(other) => MouseButton::Other(*other),
                },
            },
            W::MouseWheel { delta, .. } => Self::MouseWheel {
                delta: match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        MouseScrollDelta::LineDelta(*x, *y)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(p) => {
                        MouseScrollDelta::PixelDelta(PhysicalPosition { x: p.x, y: p.y })
                    }
                },
            },
            W::ModifiersChanged(modifiers) => {
                let m = modifiers.state();
                Self::ModifiersChanged(ModifiersState {
                    shift: m.shift_key(),
                    control: m.control_key(),
                    alt: m.alt_key(),
                    logo: m.super_key(),
                })
            }
            W::KeyboardInput { event, .. } => Self::KeyboardInput {
                event: KeyEvent {
                    physical_key: match event.physical_key {
                        winit::keyboard::PhysicalKey::Code(code) => KeyCode::from_winit(code)
                            .map_or(PhysicalKey::Unidentified, PhysicalKey::Code),
                        winit::keyboard::PhysicalKey::Unidentified(_) => PhysicalKey::Unidentified,
                    },
                    state: state(event.state),
                    repeat: event.repeat,
                },
            },
            _ => return None,
        })
    }
}
