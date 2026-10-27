use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Face {
    Bebas,
    Cormorant,
    CormorantItalic,
    Fraunces,
    FrauncesItalic,
    Alfa,
    Righteous,
    Italiana,
    Josefin,
    DMSerif,
    DMSerifItalic,
    BodoniItalic,
    Archivo,
    Bricolage,
    Della,
    Yatra,
    Rozha,
    Bangla,
    Tamil,
    Telugu,
    Thai,
    Mincho,
    Hangul,
    Unbounded,
    Cinzel,
    Pacifico,
    Ethiopic,
    Plex,
    Cjk,
}

impl Face {
    const ALL: [Face; 29] = [
        Face::Bebas,
        Face::Cormorant,
        Face::CormorantItalic,
        Face::Fraunces,
        Face::FrauncesItalic,
        Face::Alfa,
        Face::Righteous,
        Face::Italiana,
        Face::Josefin,
        Face::DMSerif,
        Face::DMSerifItalic,
        Face::BodoniItalic,
        Face::Archivo,
        Face::Bricolage,
        Face::Della,
        Face::Yatra,
        Face::Rozha,
        Face::Bangla,
        Face::Tamil,
        Face::Telugu,
        Face::Thai,
        Face::Mincho,
        Face::Hangul,
        Face::Unbounded,
        Face::Cinzel,
        Face::Pacifico,
        Face::Ethiopic,
        Face::Plex,
        Face::Cjk,
    ];

    pub fn family(self) -> FontFamily {
        FontFamily::Name(format!("arrival-{self:?}").into())
    }
}

/// The bundled CJK face, shared by the interface's fallback chain and the
/// CJK title face.
pub(crate) const CJK_FONT: &str = "Noto Sans CJK JP Regular";

pub(crate) fn cjk_bytes() -> &'static [u8] {
    vearth_platform::asset_bytes!("../../assets/fonts/NotoSansCJKjp-Regular.otf")
}

/// A title face's font file; `None` for the faces shared with the interface.
fn bytes(face: Face) -> Option<&'static [u8]> {
    Some(match face {
        Face::Bebas => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/BebasNeue-Regular.ttf")
        }
        Face::Cormorant => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/CormorantGaramond[wght].ttf")
        }
        Face::CormorantItalic => vearth_platform::asset_bytes!(
            "../../assets/fonts/titles/CormorantGaramond-Italic[wght].ttf"
        ),
        Face::Fraunces => vearth_platform::asset_bytes!(
            "../../assets/fonts/titles/Fraunces[SOFT,WONK,opsz,wght].ttf"
        ),
        Face::FrauncesItalic => vearth_platform::asset_bytes!(
            "../../assets/fonts/titles/Fraunces-Italic[SOFT,WONK,opsz,wght].ttf"
        ),
        Face::Alfa => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/AlfaSlabOne-Regular.ttf")
        }
        Face::Righteous => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/Righteous-Regular.ttf")
        }
        Face::Italiana => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/Italiana-Regular.ttf")
        }
        Face::Josefin => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/JosefinSans[wght].ttf")
        }
        Face::DMSerif => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/DMSerifDisplay-Regular.ttf")
        }
        Face::DMSerifItalic => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/DMSerifDisplay-Italic.ttf")
        }
        Face::BodoniItalic => vearth_platform::asset_bytes!(
            "../../assets/fonts/titles/BodoniModa-Italic[opsz,wght].ttf"
        ),
        Face::Archivo => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/ArchivoBlack-Regular.ttf")
        }
        Face::Bricolage => vearth_platform::asset_bytes!(
            "../../assets/fonts/titles/BricolageGrotesque[opsz,wdth,wght].ttf"
        ),
        Face::Della => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/DellaRespira-Regular.ttf")
        }
        Face::Yatra => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/YatraOne-Regular.ttf")
        }
        Face::Rozha => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/RozhaOne-Regular.ttf")
        }
        Face::Bangla => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/TiroBangla-Regular.ttf")
        }
        Face::Tamil => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/TiroTamil-Regular.ttf")
        }
        Face::Telugu => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/NotoSerifTelugu[wght].ttf")
        }
        Face::Thai => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/NotoSerifThai[wdth,wght].ttf")
        }
        Face::Mincho => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/ShipporiMincho-Regular.ttf")
        }
        Face::Hangul => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/BlackHanSans-Regular.ttf")
        }
        Face::Unbounded => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/Unbounded[wght].ttf")
        }
        Face::Cinzel => vearth_platform::asset_bytes!("../../assets/fonts/titles/Cinzel[wght].ttf"),
        Face::Pacifico => {
            vearth_platform::asset_bytes!("../../assets/fonts/titles/Pacifico-Regular.ttf")
        }
        Face::Ethiopic => vearth_platform::asset_bytes!(
            "../../assets/fonts/titles/NotoSansEthiopic[wdth,wght].ttf"
        ),
        Face::Plex | Face::Cjk => return None,
    })
}

/// Registers the title faces: every face natively; in the browser, the faces
/// downloaded so far (see [`load`]), with the rest mapped to the interface
/// font until then.
pub(crate) fn register(definitions: &mut FontDefinitions) {
    for face in Face::ALL {
        if !loaded(face) {
            definitions
                .families
                .insert(face.family(), vec!["IBM Plex Sans Regular".into()]);
            continue;
        }
        let key = match face {
            Face::Plex => "IBM Plex Sans Regular".to_owned(),
            Face::Cjk => CJK_FONT.to_owned(),
            face => {
                let key = format!("arrival-{face:?}");
                let bytes = bytes(face).expect("title face file");
                definitions
                    .font_data
                    .insert(key.clone(), Arc::new(FontData::from_static(bytes)));
                key
            }
        };
        // A dedicated face allows an atomic cmap coverage check. Unsupported
        // titles switch as a whole to the shared UI family, not glyph by glyph.
        definitions.families.insert(face.family(), vec![key]);
    }
}

/// Whether `face` can be registered now.
pub(crate) fn loaded(face: Face) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = face;
        true
    }
    #[cfg(target_arch = "wasm32")]
    {
        face == Face::Plex || web::LOADED.lock().unwrap().contains(&face)
    }
}

/// Browser: downloads `faces` that are not loaded yet, blocking the calling
/// worker, and bumps [`generation`] so the interface registers them.
#[cfg(target_arch = "wasm32")]
pub(crate) fn load(faces: &[Face]) {
    for &face in faces {
        if loaded(face) {
            continue;
        }
        match face {
            Face::Cjk => {
                cjk_bytes();
            }
            face => {
                bytes(face);
            }
        }
        web::LOADED.lock().unwrap().insert(face);
        web::GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
}

/// Browser: downloads every title face and the CJK face, for prefetching
/// once the first view has loaded.
#[cfg(target_arch = "wasm32")]
pub(crate) fn load_all() {
    load(&Face::ALL);
}

/// Changes whenever [`load`] adds a face.
#[cfg(target_arch = "wasm32")]
pub(crate) fn generation() -> u64 {
    web::GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::Face;
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU64;

    pub(super) static LOADED: Mutex<BTreeSet<Face>> = Mutex::new(BTreeSet::new());
    pub(super) static GENERATION: AtomicU64 = AtomicU64::new(0);
}
