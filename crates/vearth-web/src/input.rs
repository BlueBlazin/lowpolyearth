//! DOM input translated into the app's window events and egui events.

use vearth_app::app::web::WebApp;
use vearth_app::input::{
    ElementState, KeyCode, KeyEvent, ModifiersState, MouseButton, MouseScrollDelta, PhysicalKey,
    PhysicalPosition, PhysicalSize, WindowEvent,
};

/// `KeyboardEvent` modifier flags.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

thread_local! {
    static MODIFIERS: std::cell::Cell<Modifiers> = const {
        std::cell::Cell::new(Modifiers { shift: false, control: false, alt: false, meta: false })
    };
    static SCALE: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
}

fn is_mac() -> bool {
    use wasm_bindgen::JsCast;
    let navigator = js_sys::global()
        .unchecked_into::<web_sys::WorkerGlobalScope>()
        .navigator();
    navigator.platform().is_ok_and(|p| p.starts_with("Mac"))
}

fn egui_modifiers(m: Modifiers) -> egui::Modifiers {
    let mac = is_mac();
    egui::Modifiers {
        alt: m.alt,
        ctrl: m.control,
        shift: m.shift,
        mac_cmd: mac && m.meta,
        command: if mac { m.meta } else { m.control },
    }
}

fn sync_modifiers(app: &mut WebApp, next: Modifiers) {
    if MODIFIERS.replace(next) != next {
        app.event(
            Some(WindowEvent::ModifiersChanged(ModifiersState {
                shift: next.shift,
                control: next.control,
                alt: next.alt,
                logo: next.meta,
            })),
            Vec::new(),
        );
    }
}

pub fn resize(app: &mut WebApp, width: u32, height: u32, scale_factor: f64) {
    if SCALE.replace(scale_factor) != scale_factor {
        app.event(
            Some(WindowEvent::ScaleFactorChanged { scale_factor }),
            Vec::new(),
        );
    }
    app.event(
        Some(WindowEvent::Resized(PhysicalSize::new(
            width.max(1),
            height.max(1),
        ))),
        Vec::new(),
    );
}

pub fn pointer_move(app: &mut WebApp, x: f64, y: f64) {
    app.event(
        Some(WindowEvent::CursorMoved {
            position: PhysicalPosition { x, y },
        }),
        Vec::new(),
    );
}

pub fn pointer_button(app: &mut WebApp, button: i16, pressed: bool) {
    let button = match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        other => MouseButton::Other(other.max(0) as u16),
    };
    let state = if pressed {
        ElementState::Pressed
    } else {
        ElementState::Released
    };
    app.event(Some(WindowEvent::MouseInput { state, button }), Vec::new());
}

pub fn pointer_inside(app: &mut WebApp, inside: bool) {
    let event = if inside {
        WindowEvent::CursorEntered
    } else {
        WindowEvent::CursorLeft
    };
    app.event(Some(event), Vec::new());
}

/// DOM deltas grow downwards; winit's (and the app's) grow upwards.
pub fn wheel(app: &mut WebApp, dx: f64, dy: f64, mode: u32) {
    const DOM_DELTA_LINE: u32 = 1;
    const DOM_DELTA_PAGE: u32 = 2;
    let delta = match mode {
        DOM_DELTA_LINE => MouseScrollDelta::LineDelta(-dx as f32, -dy as f32),
        DOM_DELTA_PAGE => MouseScrollDelta::LineDelta(-dx as f32 * 20.0, -dy as f32 * 20.0),
        _ => MouseScrollDelta::PixelDelta(PhysicalPosition { x: -dx, y: -dy }),
    };
    app.event(Some(WindowEvent::MouseWheel { delta }), Vec::new());
}

pub fn focus(app: &mut WebApp, focused: bool) {
    if !focused {
        // Keys released while the page is unfocused never report keyup.
        sync_modifiers(app, Modifiers::default());
    }
    app.event(Some(WindowEvent::Focused(focused)), Vec::new());
}

/// `from_text_agent` marks keys typed into the page's text agent, whose
/// characters arrive separately as input events.
pub fn key(
    app: &mut WebApp,
    code: &str,
    key: &str,
    pressed: bool,
    repeat: bool,
    modifiers: Modifiers,
    from_text_agent: bool,
) {
    sync_modifiers(app, modifiers);
    let mut egui_events = Vec::new();
    let egui_modifiers = egui_modifiers(modifiers);
    if let Some(logical) = egui::Key::from_name(key) {
        egui_events.push(egui::Event::Key {
            key: logical,
            physical_key: egui::Key::from_name(code),
            pressed,
            repeat,
            modifiers: egui_modifiers,
        });
    }
    // Printable characters also produce text, unless they form a shortcut
    // (as in eframe: neither Ctrl nor Command held).
    if pressed
        && !from_text_agent
        && key.chars().count() == 1
        && !modifiers.control
        && !egui_modifiers.command
        && !key.chars().all(char::is_control)
    {
        egui_events.push(egui::Event::Text(key.to_owned()));
    }
    let event = KeyEvent {
        physical_key: KeyCode::from_code(code).map_or(PhysicalKey::Unidentified, PhysicalKey::Code),
        state: if pressed {
            ElementState::Pressed
        } else {
            ElementState::Released
        },
        repeat,
    };
    app.event(Some(WindowEvent::KeyboardInput { event }), egui_events);
}
