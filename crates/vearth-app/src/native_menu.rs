//! macOS system menu bar. Provider ids and enablement state come from
//! `vearth-data`, shared with the in-window UI.

#![cfg(target_os = "macos")]

use crate::app::PanSpeedPreset;
use crate::gfx::PostProcessSettings;
use crate::settings::ShadowPreference;
use muda::accelerator::{Accelerator, CMD_OR_CTRL, Code};
use muda::{AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::cell::Cell;
use vearth_data::registry::{ProviderControl, ProviderId, ProviderLayer};

#[derive(Debug)]
pub(crate) enum NativeMenuAction {
    OpenSettings,
    OpenPrivacy,
    ShowSettingsButton(bool),
    Paste,
    HidePanels(bool),
    ShowSearch(bool),
    ShowDebug(bool),
    ShowSolar(bool),
    SetDate,
    SetTwentyFourHour(bool),
    SetCloudsVisible(bool),
    SetPrecipitationVisible(bool),
    SetBuildingNightLights(bool),
    SetShadows(ShadowPreference),
    SetColorGrading(bool),
    SetFilmicSaturation(bool),
    SetWeatherEnabled(bool),
    ShowCompass(bool),
    ShowLoading(bool),
    ShowAttribution(bool),
    SetPanSpeed(PanSpeedPreset),
    SetProviderEnabled { id: ProviderId, enabled: bool },
}

/// Complete application-owned state projected into the native menu bar.
///
/// Keeping this as a value snapshot makes native menu updates idempotent:
/// rendering may inspect the application state every frame, but AppKit is
/// touched only after a semantic state change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeMenuState {
    hide_panels: bool,
    settings_button: bool,
    search: bool,
    debug: bool,
    solar: bool,
    twenty_four_hour: bool,
    clouds: bool,
    precipitation: bool,
    building_night_lights: bool,
    shadows: ShadowPreference,
    color_grading: bool,
    filmic_saturation: bool,
    weather_enabled: bool,
    compass: bool,
    loading: bool,
    attribution: bool,
    pan_speed: PanSpeedPreset,
    providers: Vec<ProviderControl>,
}

impl NativeMenuState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        panel_visibility: (bool, bool, bool, bool, bool, bool),
        hide_panels: bool,
        settings_button: bool,
        twenty_four_hour: bool,
        clouds: bool,
        precipitation: bool,
        building_night_lights: bool,
        shadows: ShadowPreference,
        post_process: PostProcessSettings,
        weather_enabled: bool,
        pan_speed: PanSpeedPreset,
        providers: Vec<ProviderControl>,
    ) -> Self {
        let (search, debug, solar, compass, loading, attribution) = panel_visibility;
        Self {
            hide_panels,
            settings_button,
            search,
            debug,
            solar,
            twenty_four_hour,
            clouds,
            precipitation,
            building_night_lights,
            shadows,
            color_grading: post_process.color_grading,
            filmic_saturation: post_process.filmic_saturation,
            weather_enabled,
            compass,
            loading,
            attribution,
            pan_speed,
            providers,
        }
    }
}

trait CheckedState {
    fn is_checked(&self) -> bool;
    fn set_checked(&self, checked: bool);
}

impl CheckedState for CheckMenuItem {
    fn is_checked(&self) -> bool {
        CheckMenuItem::is_checked(self)
    }

    fn set_checked(&self, checked: bool) {
        CheckMenuItem::set_checked(self, checked);
    }
}

/// Avoid even an idempotent `NSMenuItem.setState` call: while a popup is
/// tracking, AppKit treats it as a layout mutation.
fn apply_checked_state(item: &impl CheckedState, checked: bool) -> bool {
    if item.is_checked() == checked {
        return false;
    }
    item.set_checked(checked);
    true
}

struct PanSpeedMenuItem {
    preset: PanSpeedPreset,
    item: CheckMenuItem,
}

struct ShadowMenuItem {
    preset: ShadowPreference,
    item: CheckMenuItem,
}

fn apply_shadow_checks<'a, T: CheckedState + 'a>(
    items: impl IntoIterator<Item = (ShadowPreference, &'a T)>,
    selected: ShadowPreference,
) {
    for (preset, item) in items {
        apply_checked_state(item, preset == selected);
    }
}

struct ProviderMenuItem {
    id: ProviderId,
    item: CheckMenuItem,
}

pub(crate) struct NativeMenu {
    // Retain the whole native object graph for the application lifetime.
    _menu: Menu,
    settings: MenuItem,
    privacy: MenuItem,
    hide_panels: CheckMenuItem,
    settings_button: CheckMenuItem,
    paste: MenuItem,
    search: CheckMenuItem,
    debug: CheckMenuItem,
    solar: CheckMenuItem,
    set_date: MenuItem,
    twenty_four_hour: CheckMenuItem,
    clouds: CheckMenuItem,
    precipitation: CheckMenuItem,
    building_night_lights: CheckMenuItem,
    color_grading: CheckMenuItem,
    filmic_saturation: CheckMenuItem,
    weather_enabled: CheckMenuItem,
    compass: CheckMenuItem,
    loading: CheckMenuItem,
    attribution: CheckMenuItem,
    pan_speeds: Vec<PanSpeedMenuItem>,
    shadows: Vec<ShadowMenuItem>,
    shadow_checks_dirty: Cell<bool>,
    providers: Vec<ProviderMenuItem>,
    applied_state: NativeMenuState,
}

impl NativeMenu {
    pub(crate) fn new(initial_state: NativeMenuState) -> muda::Result<Self> {
        let settings = MenuItem::new(
            "Settings…",
            true,
            Some(Accelerator::new(Some(CMD_OR_CTRL), Code::Comma)),
        );
        let privacy = MenuItem::new("Privacy Policy…", true, None);
        let app_menu = Submenu::with_items(
            crate::platform_paths::PRODUCT_NAME,
            true,
            &[
                &PredefinedMenuItem::about(
                    Some(&format!("About {}", crate::platform_paths::PRODUCT_NAME)),
                    Some(AboutMetadata {
                        name: Some(crate::platform_paths::PRODUCT_NAME.into()),
                        version: Some(env!("CARGO_PKG_VERSION").into()),
                        copyright: Some(crate::privacy::COPYRIGHT.into()),
                        ..Default::default()
                    }),
                ),
                &PredefinedMenuItem::separator(),
                &settings,
                &privacy,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(Some(&format!(
                    "Hide {}",
                    crate::platform_paths::PRODUCT_NAME
                ))),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(Some(&format!(
                    "Quit {}",
                    crate::platform_paths::PRODUCT_NAME
                ))),
            ],
        )?;

        // A predefined macOS Paste item invokes `paste:` on Cocoa's first
        // responder and consumes Cmd+V before winit sees it. egui's TextEdit
        // is not a native responder, so use an application action and feed
        // the clipboard payload back through egui instead.
        let paste = MenuItem::new(
            "Paste",
            true,
            Some(Accelerator::new(Some(CMD_OR_CTRL), Code::KeyV)),
        );
        let edit_menu = Submenu::with_items(
            "Edit",
            true,
            &[
                &PredefinedMenuItem::undo(None),
                &PredefinedMenuItem::redo(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::cut(None),
                &PredefinedMenuItem::copy(None),
                &paste,
                &PredefinedMenuItem::select_all(None),
            ],
        )?;

        let search = CheckMenuItem::new("Search", true, initial_state.search, None);
        let settings_button =
            CheckMenuItem::new("Settings Button", true, initial_state.settings_button, None);
        let debug = CheckMenuItem::new("Debug Panel", true, initial_state.debug, None);
        let solar = CheckMenuItem::new("Solar Controls", true, initial_state.solar, None);
        let clouds = CheckMenuItem::new("Clouds", true, initial_state.clouds, None);
        let precipitation =
            CheckMenuItem::new("Precipitation", true, initial_state.precipitation, None);
        let building_night_lights = CheckMenuItem::new(
            "Building Night Lights",
            true,
            initial_state.building_night_lights,
            None,
        );
        let color_grading =
            CheckMenuItem::new("Color Grading", true, initial_state.color_grading, None);
        let filmic_saturation = CheckMenuItem::new(
            "Filmic Saturation",
            true,
            initial_state.filmic_saturation,
            None,
        );
        let visual_effects = Submenu::with_items(
            "Visual Effects",
            true,
            &[&color_grading, &filmic_saturation],
        )?;
        let compass = CheckMenuItem::new("Compass", true, initial_state.compass, None);
        let shadows_menu = Submenu::new("Shadows", true);
        let mut shadows = Vec::with_capacity(ShadowPreference::ALL.len());
        for preset in ShadowPreference::ALL {
            let item =
                CheckMenuItem::new(preset.label(), true, preset == initial_state.shadows, None);
            shadows_menu.append(&item)?;
            shadows.push(ShadowMenuItem { preset, item });
        }
        let loading = CheckMenuItem::new("Loading Progress", true, initial_state.loading, None);
        let attribution =
            CheckMenuItem::new("Data Attribution", true, initial_state.attribution, None);
        let hide_panels =
            CheckMenuItem::new("Hide UI Panels", true, initial_state.hide_panels, None);
        let view_menu = Submenu::with_items(
            "View",
            true,
            &[
                &hide_panels,
                &PredefinedMenuItem::separator(),
                &search,
                &settings_button,
                &debug,
                &solar,
                &PredefinedMenuItem::separator(),
                &clouds,
                &precipitation,
                &building_night_lights,
                &shadows_menu,
                &visual_effects,
                &PredefinedMenuItem::separator(),
                &compass,
                &loading,
                &attribution,
            ],
        )?;

        let pan_speed_menu = Submenu::new("WASD Pan Speed", true);
        let mut pan_speeds = Vec::with_capacity(PanSpeedPreset::ALL.len());
        for preset in PanSpeedPreset::ALL {
            let item = CheckMenuItem::new(
                preset.label(),
                true,
                preset == initial_state.pan_speed,
                None,
            );
            pan_speed_menu.append(&item)?;
            pan_speeds.push(PanSpeedMenuItem { preset, item });
        }
        let set_date = MenuItem::new("Set Date…", true, None);
        let twenty_four_hour =
            CheckMenuItem::new("24-Hour Time", true, initial_state.twenty_four_hour, None);
        let weather_enabled =
            CheckMenuItem::new("Weather System", true, initial_state.weather_enabled, None);
        let controls_menu = Submenu::with_items(
            "Controls",
            true,
            &[
                &set_date,
                &twenty_four_hour,
                &weather_enabled,
                &PredefinedMenuItem::separator(),
                &pan_speed_menu,
            ],
        )?;

        let data_menu = Submenu::new("Data", true);
        let mut providers = Vec::with_capacity(initial_state.providers.len());
        for layer in [
            ProviderLayer::Elevation,
            ProviderLayer::Surface,
            ProviderLayer::Imagery,
            ProviderLayer::Vectors,
        ] {
            let layer_menu = Submenu::new(layer.label(), true);
            let mut layer_len = 0;
            for control in initial_state
                .providers
                .iter()
                .filter(|control| control.id.layer == layer)
            {
                let item = CheckMenuItem::new(&control.id.name, true, control.enabled, None);
                layer_menu.append(&item)?;
                providers.push(ProviderMenuItem {
                    id: control.id.clone(),
                    item,
                });
                layer_len += 1;
            }
            if layer_len > 0 {
                data_menu.append(&layer_menu)?;
            }
        }

        let window_menu = Submenu::with_items(
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(None),
                &PredefinedMenuItem::fullscreen(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::bring_all_to_front(None),
            ],
        )?;

        let menu = Menu::with_items(&[
            &app_menu,
            &edit_menu,
            &view_menu,
            &controls_menu,
            &data_menu,
            &window_menu,
        ])?;
        menu.init_for_nsapp();
        window_menu.set_as_windows_menu_for_nsapp();

        Ok(Self {
            _menu: menu,
            hide_panels,
            settings,
            privacy,
            settings_button,
            paste,
            search,
            debug,
            solar,
            set_date,
            twenty_four_hour,
            clouds,
            precipitation,
            building_night_lights,
            color_grading,
            filmic_saturation,
            weather_enabled,
            compass,
            loading,
            attribution,
            pan_speeds,
            shadows,
            shadow_checks_dirty: Cell::new(false),
            providers,
            applied_state: initial_state,
        })
    }

    pub(crate) fn drain_actions(&self) -> Vec<NativeMenuAction> {
        let mut actions = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id() == self.settings.id() {
                actions.push(NativeMenuAction::OpenSettings);
            } else if event.id() == self.privacy.id() {
                actions.push(NativeMenuAction::OpenPrivacy);
            } else if event.id() == self.settings_button.id() {
                actions.push(NativeMenuAction::ShowSettingsButton(
                    self.settings_button.is_checked(),
                ));
            } else if event.id() == self.paste.id() {
                actions.push(NativeMenuAction::Paste);
            } else if event.id() == self.hide_panels.id() {
                actions.push(NativeMenuAction::HidePanels(self.hide_panels.is_checked()));
            } else if event.id() == self.search.id() {
                actions.push(NativeMenuAction::ShowSearch(self.search.is_checked()));
            } else if event.id() == self.debug.id() {
                actions.push(NativeMenuAction::ShowDebug(self.debug.is_checked()));
            } else if event.id() == self.solar.id() {
                actions.push(NativeMenuAction::ShowSolar(self.solar.is_checked()));
            } else if event.id() == self.set_date.id() {
                actions.push(NativeMenuAction::SetDate);
            } else if event.id() == self.twenty_four_hour.id() {
                actions.push(NativeMenuAction::SetTwentyFourHour(
                    self.twenty_four_hour.is_checked(),
                ));
            } else if event.id() == self.clouds.id() {
                actions.push(NativeMenuAction::SetCloudsVisible(self.clouds.is_checked()));
            } else if event.id() == self.precipitation.id() {
                actions.push(NativeMenuAction::SetPrecipitationVisible(
                    self.precipitation.is_checked(),
                ));
            } else if event.id() == self.building_night_lights.id() {
                actions.push(NativeMenuAction::SetBuildingNightLights(
                    self.building_night_lights.is_checked(),
                ));
            } else if event.id() == self.color_grading.id() {
                actions.push(NativeMenuAction::SetColorGrading(
                    self.color_grading.is_checked(),
                ));
            } else if event.id() == self.filmic_saturation.id() {
                actions.push(NativeMenuAction::SetFilmicSaturation(
                    self.filmic_saturation.is_checked(),
                ));
            } else if event.id() == self.weather_enabled.id() {
                actions.push(NativeMenuAction::SetWeatherEnabled(
                    self.weather_enabled.is_checked(),
                ));
            } else if event.id() == self.compass.id() {
                actions.push(NativeMenuAction::ShowCompass(self.compass.is_checked()));
            } else if event.id() == self.loading.id() {
                actions.push(NativeMenuAction::ShowLoading(self.loading.is_checked()));
            } else if event.id() == self.attribution.id() {
                actions.push(NativeMenuAction::ShowAttribution(
                    self.attribution.is_checked(),
                ));
            } else if let Some(shadow) = self
                .shadows
                .iter()
                .find(|shadow| event.id() == shadow.item.id())
            {
                self.shadow_checks_dirty.set(true);
                actions.push(NativeMenuAction::SetShadows(shadow.preset));
            } else if let Some(speed) = self
                .pan_speeds
                .iter()
                .find(|speed| event.id() == speed.item.id())
            {
                actions.push(NativeMenuAction::SetPanSpeed(speed.preset));
            } else if let Some(provider) = self
                .providers
                .iter()
                .find(|provider| event.id() == provider.item.id())
            {
                actions.push(NativeMenuAction::SetProviderEnabled {
                    id: provider.id.clone(),
                    enabled: provider.item.is_checked(),
                });
            }
        }
        actions
    }

    pub(crate) fn apply_state(&mut self, state: NativeMenuState) {
        // Clicking the selected check item can uncheck it without changing
        // application state. Restore the exclusive selection even in that case.
        let shadow_checks_dirty = self.shadow_checks_dirty.replace(false);
        if self.applied_state == state && !shadow_checks_dirty {
            return;
        }
        apply_shadow_checks(
            self.shadows
                .iter()
                .map(|shadow| (shadow.preset, &shadow.item)),
            state.shadows,
        );
        apply_checked_state(&self.hide_panels, state.hide_panels);
        apply_checked_state(&self.search, state.search);
        apply_checked_state(&self.settings_button, state.settings_button);
        apply_checked_state(&self.debug, state.debug);
        apply_checked_state(&self.solar, state.solar);
        apply_checked_state(&self.twenty_four_hour, state.twenty_four_hour);
        apply_checked_state(&self.clouds, state.clouds);
        apply_checked_state(&self.precipitation, state.precipitation);
        apply_checked_state(&self.building_night_lights, state.building_night_lights);
        apply_checked_state(&self.color_grading, state.color_grading);
        apply_checked_state(&self.filmic_saturation, state.filmic_saturation);
        apply_checked_state(&self.weather_enabled, state.weather_enabled);
        apply_checked_state(&self.compass, state.compass);
        apply_checked_state(&self.loading, state.loading);
        apply_checked_state(&self.attribution, state.attribution);
        for speed in &self.pan_speeds {
            apply_checked_state(&speed.item, speed.preset == state.pan_speed);
        }
        for provider in &self.providers {
            if let Some(control) = state
                .providers
                .iter()
                .find(|control| control.id == provider.id)
            {
                apply_checked_state(&provider.item, control.enabled);
            }
        }

        self.applied_state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckedState, ShadowPreference, apply_checked_state, apply_shadow_checks};
    use std::cell::Cell;

    #[derive(Default)]
    struct TestCheckItem {
        checked: Cell<bool>,
        writes: Cell<usize>,
    }

    impl CheckedState for TestCheckItem {
        fn is_checked(&self) -> bool {
            self.checked.get()
        }

        fn set_checked(&self, checked: bool) {
            self.checked.set(checked);
            self.writes.set(self.writes.get() + 1);
        }
    }

    #[test]
    fn unchanged_check_state_performs_no_native_write() {
        let item = TestCheckItem::default();

        assert!(!apply_checked_state(&item, false));
        assert_eq!(item.writes.get(), 0);
    }

    #[test]
    fn changed_check_state_is_written_exactly_once() {
        let item = TestCheckItem::default();

        assert!(apply_checked_state(&item, true));
        assert!(item.checked.get());
        assert_eq!(item.writes.get(), 1);
        assert!(!apply_checked_state(&item, true));
        assert_eq!(item.writes.get(), 1);
    }

    #[test]
    fn shadow_selection_is_exclusive_and_restored_after_reselecting() {
        let items = ShadowPreference::ALL.map(|preset| (preset, TestCheckItem::default()));
        for selected in [
            ShadowPreference::Normal,
            ShadowPreference::Dark,
            ShadowPreference::Off,
        ] {
            let sync = || apply_shadow_checks(items.iter().map(|(p, i)| (*p, i)), selected);
            sync();
            for (preset, item) in &items {
                assert_eq!(item.is_checked(), *preset == selected);
            }
            let writes = items.each_ref().map(|(_, item)| item.writes.get());
            sync();
            assert_eq!(items.each_ref().map(|(_, item)| item.writes.get()), writes);

            let (_, clicked) = items
                .iter()
                .find(|(preset, _)| *preset == selected)
                .unwrap();
            clicked.checked.set(false);
            sync();
            assert!(clicked.is_checked());
            assert_eq!(
                items.iter().filter(|(_, item)| item.is_checked()).count(),
                1
            );
        }
    }
}
