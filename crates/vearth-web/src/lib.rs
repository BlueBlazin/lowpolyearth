//! Browser entry point for Low Poly Earth.
//!
//! `web/app-worker.js` instantiates this module in the root worker, which owns
//! the GPU device and application state, and forwards the page's DOM events
//! to the functions below. See `vearth_platform::web` for the worker model.

#![cfg(target_arch = "wasm32")]

mod input;
mod selftest;

use std::cell::RefCell;
use std::rc::Rc;
use vearth_app::app::StartPos;
use vearth_app::app::web::{WebApp, schedule_frame};
use vearth_app::input::PhysicalSize;
use vearth_app::solar::SolarStart;
use wasm_bindgen::prelude::*;

pub use selftest::run_platform_selftest;

/// The system allocator serializes every worker on one lock.
#[global_allocator]
static ALLOCATOR: vearth_platform::alloc::ThreadCachingAlloc =
    vearth_platform::alloc::ThreadCachingAlloc;

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<WebApp>>>> = const { RefCell::new(None) };
}

fn with_app(f: impl FnOnce(&mut WebApp)) {
    APP.with_borrow(|app| {
        if let Some(app) = app {
            f(&mut app.borrow_mut());
            schedule_frame(app);
        }
    });
}

/// Installs logging and panic reporting for every thread. Called once by the
/// root worker before anything else.
#[wasm_bindgen]
pub fn init_runtime() {
    vearth_platform::web::install_console_logger();
    vearth_platform::web::thread::set_current_name("vearth-main");
}

/// Points asset loading at the site's `assets/` directory and its manifest.
#[wasm_bindgen]
pub fn configure_site(asset_base: String, manifest_json: String) -> Result<(), JsError> {
    vearth_platform::assets::configure(asset_base, &manifest_json).map_err(|e| JsError::new(&e))
}

/// Starts the app on a transferred canvas. `query` is the page's URL query,
/// which accepts the native command line's start options:
/// `at=lat,lon[,alt[,heading[,pitch]]]`, `time=live|<UTC date-time>`,
/// `weather=<mode or preset>` and `play`. `VEARTH_*` parameters set the
/// native build's diagnostic environment variables, and `RUST_LOG` its log
/// filter.
#[wasm_bindgen]
pub fn start(
    canvas: web_sys::OffscreenCanvas,
    width: u32,
    height: u32,
    scale_factor: f64,
    query: String,
) {
    vearth_platform::env::set_vars(
        query_pairs(&query).filter(|(key, _)| key.starts_with("VEARTH_")),
    );
    if let Some((_, filter)) = query_pairs(&query).find(|(key, _)| key == "RUST_LOG") {
        vearth_platform::web::set_log_filter(&filter);
    }
    let app = WebApp::start(
        canvas,
        PhysicalSize::new(width.max(1), height.max(1)),
        scale_factor,
        start_pos(&query),
    );
    APP.with_borrow_mut(|slot| *slot = Some(app));
}

/// Decoded `key=value` pairs of a URL query.
fn query_pairs(query: &str) -> impl Iterator<Item = (String, String)> + '_ {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |text: &str| {
                js_sys::decode_uri_component(text)
                    .ok()
                    .and_then(|text| text.as_string())
                    .unwrap_or_default()
            };
            (decode(key), decode(value))
        })
}

fn start_pos(query: &str) -> StartPos {
    let mut start = StartPos::default();
    for (key, value) in query_pairs(query) {
        match key.as_str() {
            "at" => {
                let parts: Vec<f64> = value
                    .split(',')
                    .filter_map(|p| p.trim().parse().ok())
                    .collect();
                if parts.len() >= 2 {
                    start.lat_deg = parts[0];
                    start.lon_deg = parts[1];
                    start.alt_m = parts.get(2).copied().unwrap_or(2_000.0);
                    start.heading_deg = parts.get(3).copied().unwrap_or(0.0);
                    start.pitch_deg = parts.get(4).copied().unwrap_or(0.0);
                } else {
                    log::warn!("at expects lat,lon[,alt[,heading[,pitch]]]");
                }
            }
            "time" if value.eq_ignore_ascii_case("live") => start.solar_start = SolarStart::Live,
            "time" => match vearth_app::solar::parse_utc_datetime(&value) {
                Ok(seconds) => start.solar_start = SolarStart::At(seconds),
                Err(error) => log::warn!("invalid time {value:?}: {error}"),
            },
            "weather" => match vearth_app::weather::WeatherConfig::parse_mode(&value) {
                Ok((mode, preset)) => {
                    start.weather.mode = mode;
                    start.weather.preset = preset;
                }
                Err(error) => log::warn!("{error}"),
            },
            "play" => start.play = true,
            _ => {}
        }
    }
    start
}

/// The canvas's size in device pixels, or the display's scale, changed.
#[wasm_bindgen]
pub fn resize(width: u32, height: u32, scale_factor: f64) {
    with_app(|app| input::resize(app, width, height, scale_factor));
}

#[wasm_bindgen]
pub fn pointer_move(x: f64, y: f64) {
    with_app(|app| input::pointer_move(app, x, y));
}

#[wasm_bindgen]
pub fn pointer_button(button: i16, pressed: bool) {
    with_app(|app| input::pointer_button(app, button, pressed));
}

#[wasm_bindgen]
pub fn pointer_inside(inside: bool) {
    with_app(|app| input::pointer_inside(app, inside));
}

/// `mode` is `WheelEvent.deltaMode`; pixel deltas are in device pixels.
#[wasm_bindgen]
pub fn wheel(dx: f64, dy: f64, mode: u32) {
    with_app(|app| input::wheel(app, dx, dy, mode));
}

/// A `keydown`/`keyup` with the `KeyboardEvent` fields the app needs.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn key(
    code: String,
    key: String,
    pressed: bool,
    repeat: bool,
    shift: bool,
    control: bool,
    alt: bool,
    meta: bool,
    from_text_agent: bool,
) {
    with_app(|app| {
        input::key(
            app,
            &code,
            &key,
            pressed,
            repeat,
            input::Modifiers {
                shift,
                control,
                alt,
                meta,
            },
            from_text_agent,
        )
    });
}

/// Text typed into the page's text agent.
#[wasm_bindgen]
pub fn text(text: String) {
    with_app(|app| app.event(None, vec![egui::Event::Text(text)]));
}

/// An input-method composition in progress; `committed` ends it.
#[wasm_bindgen]
pub fn composition(text: String, committed: bool) {
    let event = if committed {
        egui::ImeEvent::Commit(text)
    } else {
        egui::ImeEvent::Preedit {
            text,
            active_range_chars: None,
        }
    };
    with_app(|app| app.event(None, vec![egui::Event::Ime(event)]));
}

#[wasm_bindgen]
pub fn paste(text: String) {
    with_app(|app| app.event(None, vec![egui::Event::Paste(text)]));
}

/// The page's copy or cut command; egui answers with text to put on the
/// clipboard.
#[wasm_bindgen]
pub fn clipboard_command(cut: bool) {
    let event = if cut {
        egui::Event::Cut
    } else {
        egui::Event::Copy
    };
    with_app(|app| app.event(None, vec![event]));
}

#[wasm_bindgen]
pub fn focus(focused: bool) {
    with_app(|app| input::focus(app, focused));
}

#[wasm_bindgen]
pub fn visibility(hidden: bool) {
    with_app(|app| app.set_hidden(hidden));
}

#[wasm_bindgen]
pub fn shutdown() {
    with_app(WebApp::shutdown);
}
