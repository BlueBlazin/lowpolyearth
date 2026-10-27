//! Geographic arrival titles. Composition is fixed throughout each reveal;
//! elapsed time changes ink opacity and color, never text position or tracking.

pub mod catalog;
mod fonts;

use catalog::{Case, Finish, Layout, Profile, Reveal};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, pos2, vec2};
use std::sync::Arc;
use vearth_data::vectors::NamedLocation;

pub(crate) use fonts::register as register_fonts;
pub(crate) use fonts::{CJK_FONT, cjk_bytes};
#[cfg(target_arch = "wasm32")]
pub(crate) use fonts::{
    Face, generation as font_generation, load as load_faces, load_all as load_all_faces,
    loaded as face_loaded,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ArrivalTitle {
    pub profile: &'static Profile,
    pub primary: String,
    pub region: Option<String>,
    pub localized_names: Vec<(String, String)>,
}

/// The title style for a position in radians.
fn profile_at(position: Option<(f64, f64)>) -> &'static Profile {
    position.map_or(&catalog::FALLBACK, |(lat, lon)| {
        let context = vearth_data::admin_regions::AdminRegionAtlas::bundled().context_at(lat, lon);
        catalog::select(
            context.map(|c| c.country_name),
            lat.to_degrees(),
            lon.to_degrees(),
        )
    })
}

/// Browser: downloads the faces a title at `position` uses, blocking the
/// calling worker, so the title can start with them. Native builds embed
/// every face.
pub fn prepare_fonts(position: Option<(f64, f64)>) {
    #[cfg(target_arch = "wasm32")]
    {
        let profile = profile_at(position);
        fonts::load(&[profile.face, profile.local_face]);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = position;
}

impl ArrivalTitle {
    pub fn new(location: &NamedLocation, position: Option<(f64, f64)>) -> Self {
        let profile = profile_at(position);
        let primary = location
            .primary_name
            .as_ref()
            .unwrap_or(&location.name)
            .clone();
        let region = location
            .primary_name
            .as_ref()
            .and(location.region_name.clone());
        Self {
            profile,
            primary,
            region,
            localized_names: location.localized_names.clone(),
        }
    }
}

/// Native GPU proofs and production use this same layout and painting path.
#[derive(Debug, Clone)]
pub struct TitleLayout {
    pub text: String,
    pub native: bool,
    pub main: Arc<Galley>,
    pub main_pos: Pos2,
    pub support: Vec<(Arc<Galley>, Pos2)>,
    pub bounds: Rect,
    pub scale: f32,
}

fn ltr_supported(text: &str) -> bool {
    !text.chars().any(|c| {
        c.is_control()
            || matches!(c as u32,
        0x0590..=0x08ff | 0xfb1d..=0xfdff | 0xfe70..=0xfeff |
        0x10800..=0x10fff | 0x1e800..=0x1eeff | 0xe000..=0xf8ff |
        0xf0000..=0xffffd | 0x100000..=0x10fffd | 0x202a..=0x202e | 0x2066..=0x2069)
    })
}

fn local_script_matches(language: &str, text: &str) -> bool {
    let in_range = |start, end| text.chars().any(|c| (start..=end).contains(&(c as u32)));
    match language {
        "hi" | "mr" | "ne" => in_range(0x0900, 0x097f),
        "bn" => in_range(0x0980, 0x09ff),
        "ta" => in_range(0x0b80, 0x0bff),
        "te" => in_range(0x0c00, 0x0c7f),
        "th" => in_range(0x0e00, 0x0e7f),
        "am" => in_range(0x1200, 0x139f),
        "ja" => in_range(0x3040, 0x30ff) || in_range(0x3400, 0x9fff),
        "zh" => in_range(0x3400, 0x9fff),
        "ko" => in_range(0xac00, 0xd7af) || in_range(0x1100, 0x11ff),
        "el" => in_range(0x0370, 0x03ff),
        _ => text
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(|c| (c as u32) <= 0x024f || matches!(c as u32, 0x1e00..=0x1eff)),
    }
}

fn coverage(ctx: &egui::Context, family: FontFamily, text: &str) -> bool {
    use skrifa::MetadataProvider;
    ltr_supported(text)
        && ctx.fonts_mut(|fonts| {
            let definitions = fonts.definitions();
            let Some(faces) = definitions.families.get(&family) else {
                return false;
            };
            text.chars().all(|c| {
                // Joiners affect shaping and intentionally have no visible glyph.
                matches!(c, '\u{200c}' | '\u{200d}')
                    || faces.iter().any(|key| {
                        definitions
                            .font_data
                            .get(key)
                            .and_then(|data| {
                                skrifa::FontRef::from_index(data.font.as_ref(), data.index).ok()
                            })
                            .is_some_and(|font| font.charmap().map(c).is_some())
                    })
            })
        })
}

fn display_text(ctx: &egui::Context, title: &ArrivalTitle) -> (String, FontFamily, bool) {
    let p = title.profile;
    if !p.language.is_empty() {
        for language in [p.language, ""] {
            if let Some((_, name)) = title.localized_names.iter().find(|(lang, name)| {
                lang == language
                    && (name != &title.primary || name.chars().any(|c| c as u32 >= 0x0370))
                    && local_script_matches(p.language, name)
                    && coverage(ctx, p.local_face.family(), name)
            }) {
                let text = if p.case == Case::Upper && !name.chars().any(|c| c as u32 >= 0x0370) {
                    name.to_uppercase()
                } else {
                    name.clone()
                };
                if coverage(ctx, p.local_face.family(), &text) {
                    return (text, p.local_face.family(), true);
                }
            }
        }
    }
    let text = if ltr_supported(&title.primary) {
        title.primary.clone()
    } else {
        title
            .region
            .clone()
            .filter(|s| ltr_supported(s))
            .unwrap_or_default()
    };
    let styled = if p.case == Case::Upper {
        text.to_uppercase()
    } else {
        text
    };
    if coverage(ctx, p.face.family(), &styled) {
        return (styled, p.face.family(), false);
    }
    for family in [
        FontFamily::Proportional,
        fonts::Face::Yatra.family(),
        fonts::Face::Bangla.family(),
        fonts::Face::Tamil.family(),
        fonts::Face::Telugu.family(),
        fonts::Face::Thai.family(),
        fonts::Face::Ethiopic.family(),
    ] {
        if coverage(ctx, family.clone(), &styled) {
            return (styled, family, false);
        }
    }
    let region = title
        .region
        .as_ref()
        .filter(|name| coverage(ctx, FontFamily::Proportional, name))
        .cloned()
        .unwrap_or_default();
    (region, FontFamily::Proportional, false)
}

fn stacked(text: &str) -> String {
    // Break at a word boundary close to the center. Never split a Unicode
    // cluster, an unspaced script, or the letters of an individual word.
    let midpoint = text.chars().count() / 2;
    text.char_indices()
        .enumerate()
        .filter(|(_, (_, c))| *c == ' ')
        .min_by_key(|(index, _)| index.abs_diff(midpoint))
        .map_or_else(
            || text.to_owned(),
            |(_, (byte, _))| format!("{}\n{}", &text[..byte], &text[byte + 1..]),
        )
}

fn galley(
    ctx: &egui::Context,
    text: &str,
    family: FontFamily,
    size: f32,
    tracking: f32,
    weight: f32,
    width: f32,
    align: egui::Align,
) -> Arc<Galley> {
    let format = TextFormat {
        font_id: FontId::new(size, family),
        color: Color32::WHITE,
        extra_letter_spacing: tracking * size,
        coords: egui::epaint::text::VariationCoords::new([("wght", weight), ("opsz", size)]),
        ..Default::default()
    };
    let mut job = LayoutJob::single_section(text.to_owned(), format);
    job.halign = align;
    job.wrap.max_width = width;
    job.wrap.max_rows = 2;
    ctx.fonts_mut(|f| f.layout_job(job))
}

pub fn layout(ctx: &egui::Context, title: &ArrivalTitle, viewport: Rect) -> TitleLayout {
    let p = title.profile;
    let scale = (viewport.height() / 900.).clamp(0.45, 1.15);
    let (text, family, native) = display_text(ctx, title);
    let tracking = if text.chars().any(|c| c as u32 >= 0x0370) {
        0.
    } else {
        p.type_settings[2]
    };
    let text = if p.layout == Layout::Stack && !native {
        stacked(&text)
    } else {
        text
    };
    // Keep every profile centered so arrivals stay clear of the corner HUD.
    let align = egui::Align::Center;
    let width = (viewport.width()
        * if p.layout == Layout::Intimate {
            0.66
        } else {
            0.84
        })
    .max(1.);
    let mut size = p.type_settings[0] * scale;
    let height_budget = (viewport.height() * 0.26).max(1.);
    let mut main = galley(
        ctx,
        &text,
        family.clone(),
        size,
        tracking,
        p.type_settings[1],
        f32::INFINITY,
        align,
    );
    // Measure the complete title before wrapping. Font size depends solely on
    // the title and viewport, so every animation frame uses identical geometry.
    let fit = (width / main.rect.width().max(1.))
        .min(height_budget / main.rect.height().max(1.))
        .min(1.);
    size *= fit.max(0.48);
    main = galley(
        ctx,
        &text,
        family.clone(),
        size,
        tracking,
        p.type_settings[1],
        width,
        align,
    );
    if main.rect.height() > height_budget {
        size *= height_budget / main.rect.height();
        main = galley(
            ctx,
            &text,
            family,
            size,
            tracking,
            p.type_settings[1],
            width,
            align,
        );
    }
    let mut captions = Vec::new();
    if native && text != title.primary && ltr_supported(&title.primary) {
        captions.push(title.primary.clone());
    }
    if let Some(region) = &title.region
        && ltr_supported(region)
        && !captions.contains(region)
        && region != &title.primary
        && !region.eq_ignore_ascii_case(&text.replace('\n', " "))
    {
        captions.push(region.clone());
    }
    let caption_size = (15. * scale).max(10.);
    let gap = 8. * scale;
    let support: Vec<_> = captions
        .iter()
        .map(|caption| {
            galley(
                ctx,
                caption,
                FontFamily::Proportional,
                caption_size,
                0.075,
                400.,
                width,
                align,
            )
        })
        .collect();
    let total_height =
        main.rect.height() + support.iter().map(|g| g.rect.height() + gap).sum::<f32>();
    let bottom = viewport.bottom() - (viewport.height() * 0.115).clamp(48., 110.);
    let x = viewport.center().x;
    let position = |g: &Galley, y| pos2(x - g.rect.width() / 2., y) - g.rect.min.to_vec2();
    let main_pos = position(&main, bottom - total_height);
    let mut bounds = main.rect.translate(main_pos.to_vec2());
    let mut y = bottom - total_height + main.rect.height() + gap;
    let support = support
        .into_iter()
        .map(|g| {
            let pos = position(&g, y);
            bounds = bounds.union(g.rect.translate(pos.to_vec2()));
            y += g.rect.height() + gap;
            (g, pos)
        })
        .collect();
    TitleLayout {
        text,
        native,
        main,
        main_pos,
        support,
        bounds,
        scale,
    }
}

fn ink(rgb: u32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
        (alpha.clamp(0., 1.) * 255.).round() as u8,
    )
}

fn scrim(painter: &Painter, rect: Rect, opacity: f32) {
    let mut mesh = egui::Mesh::default();
    for row in 0..=8 {
        let y = row as f32 / 8.;
        for col in 0..=8 {
            let x = col as f32 / 8.;
            let a = (std::f32::consts::PI * x).sin().powi(2)
                * (std::f32::consts::PI * y).sin().powi(2)
                * opacity;
            mesh.colored_vertex(
                pos2(
                    rect.left() + rect.width() * x,
                    rect.top() + rect.height() * y,
                ),
                ink(0x050a10, a),
            );
            if row < 8 && col < 8 {
                let i = row * 9 + col;
                mesh.add_triangle(i, i + 1, i + 9);
                mesh.add_triangle(i + 1, i + 10, i + 9);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn text_shadow(painter: &Painter, pos: Pos2, g: &Arc<Galley>, scale: f32, alpha: f32) {
    for radius in [1.5, 3.0, 5.0] {
        for offset in [
            vec2(-1., 0.),
            vec2(1., 0.),
            vec2(0., -1.),
            vec2(0., 1.),
            vec2(-0.7, -0.7),
            vec2(0.7, 0.7),
            vec2(-0.7, 0.7),
            vec2(0.7, -0.7),
        ] {
            painter.galley_with_override_text_color(
                pos + offset * (radius * scale),
                g.clone(),
                ink(0x03080c, alpha * 0.035),
            );
        }
    }
    painter.galley_with_override_text_color(
        pos + vec2(0., 1.2) * scale,
        g.clone(),
        ink(0x03080c, alpha * 0.28),
    );
}

pub fn paint(ctx: &egui::Context, title: &ArrivalTitle, elapsed: f32, alpha: f32) {
    let viewport = ctx.content_rect();
    if alpha <= 0. || viewport.width() < 100. || viewport.height() < 160. {
        return;
    }
    let p = title.profile;
    let geometry = layout(ctx, title, viewport);
    if geometry.text.is_empty() {
        return;
    }
    let painter = ctx
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("location arrival title"),
        ))
        .with_clip_rect(viewport);
    let alpha = alpha.clamp(0., 1.);
    let scale = geometry.scale;
    let pos = geometry.main_pos;
    let g = &geometry.main;
    scrim(
        &painter,
        geometry.bounds.expand2(vec2(90., 55.) * scale),
        alpha * 0.43,
    );
    text_shadow(&painter, pos, g, scale, alpha);
    let developed = smoothstep((elapsed - 0.1) / 0.8);
    let finish_alpha = alpha
        * match p.reveal {
            Reveal::Print => 1.,
            _ => developed,
        };
    match p.finish {
        Finish::Press => {
            for step in (1..=4).rev() {
                painter.galley_with_override_text_color(
                    pos + vec2(0.65, 0.85) * (step as f32 * scale),
                    g.clone(),
                    ink(p.accent, finish_alpha * 0.9),
                );
            }
        }
        Finish::Edge => {
            for offset in [
                vec2(-1., 0.),
                vec2(1., 0.),
                vec2(0., -1.),
                vec2(0., 1.),
                vec2(-0.7, -0.7),
                vec2(0.7, 0.7),
                vec2(-0.7, 0.7),
                vec2(0.7, -0.7),
            ] {
                painter.galley_with_override_text_color(
                    pos + offset * (1.4 * scale),
                    g.clone(),
                    ink(p.accent, finish_alpha * 0.8),
                );
            }
        }
        Finish::Glow => {
            for radius in [3., 6., 10.] {
                for offset in [vec2(-1., 0.), vec2(1., 0.), vec2(0., -1.), vec2(0., 1.)] {
                    painter.galley_with_override_text_color(
                        pos + offset * (radius * scale),
                        g.clone(),
                        ink(p.accent, finish_alpha * 0.025),
                    );
                }
            }
        }
        _ => {}
    }
    painter.galley_with_override_text_color(pos, g.clone(), ink(p.ink, alpha));
    if p.finish == Finish::SplitInk {
        // Color the lower line (or lower portion of a single unspaced name),
        // preserving one shaped galley and all of its cluster positions.
        let rect = g.rect.translate(pos.to_vec2());
        let split = if g.rows.len() > 1 {
            rect.top() + g.rows[1].pos.y
        } else {
            rect.center().y + rect.height() * 0.12
        };
        let clip = Rect::from_min_max(
            pos2(rect.left() - 4., split),
            pos2(rect.right() + 4., rect.bottom() + 4.),
        );
        painter
            .with_clip_rect(clip.intersect(viewport))
            .galley_with_override_text_color(pos, g.clone(), ink(p.accent, finish_alpha));
    }
    let caption_alpha = alpha * smoothstep((elapsed - 0.25) / 0.65);
    for (g, pos) in &geometry.support {
        text_shadow(&painter, *pos, g, scale, caption_alpha);
        painter.galley_with_override_text_color(
            *pos,
            g.clone(),
            ink(0xe4e3d9, caption_alpha * 0.9),
        );
    }
    ctx.request_repaint();
}

pub(crate) fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

#[cfg(test)]
mod tests;
