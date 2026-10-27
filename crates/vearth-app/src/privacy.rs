//! Offline privacy policy shared with the packaged and publishable documents.

pub const POLICY: &str = include_str!("../../../docs/PRIVACY.md");
pub const COPYRIGHT: &str = "Copyright © 2026 BlueBlazin.";

pub(crate) fn show(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("Privacy Policy")
        .open(open)
        .default_width(560.0)
        .max_height((ctx.content_rect().height() - 48.0).max(240.0))
        .collapsible(false)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for paragraph in POLICY.trim().split("\n\n") {
                    if let Some(title) = paragraph.strip_prefix("# ") {
                        ui.heading(title);
                    } else if let Some(title) = paragraph.strip_prefix("## ") {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(title).strong());
                    } else {
                        ui.label(paragraph.replace('\n', " "));
                    }
                    ui.add_space(6.0);
                }
            });
        });
}
