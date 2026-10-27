use super::fonts::Face;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layout {
    Center,
    Left,
    Right,
    Stack,
    Wide,
    Intimate,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Case {
    Source,
    Upper,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Finish {
    Clean,
    Press,
    Edge,
    Glow,
    SplitInk,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reveal {
    Dissolve,
    Develop,
    Print,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub key: &'static str,
    pub countries: &'static str,
    /// Degrees and influence radius in km; zero radius means country fallback.
    pub center: [f64; 3],
    pub face: Face,
    /// Size in logical points, variable weight, tracking in em.
    pub type_settings: [f32; 3],
    pub layout: Layout,
    pub case: Case,
    pub finish: Finish,
    pub ink: u32,
    pub accent: u32,
    pub language: &'static str,
    pub local_face: Face,
    pub reveal: Reveal,
}

macro_rules! p {
    ($key:literal, $countries:literal, $center:expr, $face:ident, $settings:expr,
     $layout:ident, $case:ident, $finish:ident, $ink:literal, $accent:literal,
     $lang:literal, $local:ident, $reveal:ident) => {
        Profile {
            key: $key,
            countries: $countries,
            center: $center,
            face: Face::$face,
            type_settings: $settings,
            layout: Layout::$layout,
            case: Case::$case,
            finish: Finish::$finish,
            ink: $ink,
            accent: $accent,
            language: $lang,
            local_face: Face::$local,
            reveal: Reveal::$reveal,
        }
    };
}

pub const FALLBACK: Profile = p!(
    "world",
    "",
    [0., 0., 0.],
    Cormorant,
    [66., 500., 0.025],
    Center,
    Source,
    Clean,
    0xf6eee0,
    0xc8d1d0,
    "",
    Plex,
    Dissolve
);

// The radii bound art direction, not place names or political boundaries.
// Country guards prevent nearby border cities exporting their local treatment.
// A profile never substitutes its key or a hand-authored translation for data.
#[rustfmt::skip]
pub const PROFILES: &[Profile] = &[
    // North America: editorial, cinema, neon, adobe, and maritime lettering.
    p!("new-york", "United States of America", [40.713,-74.006,48.], Bebas, [86.,400.,0.035], Wide, Upper, Clean, 0xf6f1e6, 0xbdc8d0, "", Plex, Print),
    p!("los-angeles", "United States of America", [34.052,-118.244,62.], Italiana, [82.,400.,0.0], Center, Source, Glow, 0xffe8c0, 0xd89975, "", Plex, Develop),
    p!("san-francisco", "United States of America", [37.775,-122.419,32.], Fraunces, [65.,600.,-0.025], Stack, Source, Clean, 0xf0d8bd, 0x98bdb5, "", Plex, Dissolve),
    p!("chicago", "United States of America", [41.878,-87.63,42.], Unbounded, [56.,600.,-0.01], Left, Upper, Edge, 0xd7e9ef, 0x506781, "", Plex, Print),
    p!("new-orleans", "United States of America", [29.951,-90.072,35.], DMSerifItalic, [72.,400.,0.0], Stack, Source, Press, 0xe9c878, 0x663e62, "", Plex, Develop),
    p!("miami", "United States of America", [25.762,-80.192,40.], Josefin, [76.,500.,0.13], Wide, Upper, Glow, 0xffd5d1, 0x75bec3, "", Plex, Dissolve),
    p!("santa-fe", "United States of America", [35.687,-105.938,60.], Della, [67.,400.,0.055], Center, Upper, Press, 0xeccc98, 0x854d3c, "", Plex, Develop),
    p!("pacific-northwest", "United States of America|Canada", [47.606,-122.332,190.], CormorantItalic, [68.,500.,0.0], Left, Source, Clean, 0xc9e3d6, 0x8aaba3, "", Plex, Dissolve),
    p!("montreal", "Canada", [45.502,-73.567,40.], Bricolage, [64.,600.,-0.025], Left, Source, Clean, 0xf5dec9, 0xd1866d, "fr", Bricolage, Print),
    p!("quebec", "Canada", [46.814,-71.208,50.], Cormorant, [73.,600.,0.015], Center, Source, Edge, 0xe9e8d8, 0x7d98b1, "fr", Cormorant, Develop),
    p!("mexico-city", "Mexico", [19.433,-99.133,48.], Bricolage, [73.,800.,-0.035], Stack, Upper, Press, 0xf6d9a1, 0xa64942, "es", Bricolage, Print),
    p!("oaxaca", "Mexico", [17.074,-96.727,80.], Fraunces, [74.,650.,-0.01], Center, Source, Press, 0xeac27e, 0x916545, "es", Fraunces, Develop),
    p!("havana", "Cuba", [23.113,-82.367,45.], Pacifico, [65.,400.,0.0], Right, Source, Press, 0xbce3d5, 0x376c77, "es", Pacifico, Dissolve),
    p!("san-juan", "Puerto Rico", [18.466,-66.106,32.], DMSerif, [70.,400.,0.015], Center, Source, Edge, 0xf5dfa3, 0x9b5663, "es", DMSerif, Develop),
    // South America: Brazilian geometric warmth, Andean print, Río de la Plata contrast.
    p!("rio", "Brazil", [-22.907,-43.173,45.], Righteous, [79.,400.,0.0], Stack, Source, SplitInk, 0xb7ead0, 0xf6bb92, "pt", Righteous, Print),
    p!("sao-paulo", "Brazil", [-23.551,-46.634,55.], Archivo, [69.,400.,-0.025], Left, Upper, Clean, 0xf2eee6, 0xb9b5a9, "pt", Archivo, Print),
    p!("salvador", "Brazil", [-12.978,-38.502,40.], FrauncesItalic, [76.,650.,-0.02], Center, Source, Press, 0xf1d480, 0x807b9b, "pt", FrauncesItalic, Develop),
    p!("brasilia", "Brazil", [-15.794,-47.882,65.], Josefin, [71.,500.,0.115], Wide, Upper, Clean, 0xefe9d5, 0xc49d65, "pt", Josefin, Print),
    p!("manaus", "Brazil", [-3.119,-60.022,140.], DMSerif, [70.,400.,0.0], Intimate, Source, Glow, 0xcce6b7, 0x638d70, "pt", DMSerif, Dissolve),
    p!("buenos-aires", "Argentina", [-34.604,-58.382,55.], BodoniItalic, [82.,500.,-0.01], Stack, Source, Clean, 0xeee4d4, 0xb3c9d2, "es", BodoniItalic, Develop),
    p!("patagonia", "Argentina|Chile", [-49.3,-72.9,550.], Cormorant, [68.,400.,0.14], Wide, Upper, Clean, 0xd9e8e9, 0x9aaebb, "es", Cormorant, Dissolve),
    p!("santiago", "Chile", [-33.449,-70.669,45.], Unbounded, [53.,500.,0.005], Left, Upper, Clean, 0xe7ded2, 0xb29a89, "es", Unbounded, Print),
    p!("valparaiso", "Chile", [-33.047,-71.613,28.], Righteous, [69.,400.,0.02], Right, Source, Press, 0xf6cf9c, 0x5b7ba0, "es", Righteous, Develop),
    p!("lima", "Peru", [-12.046,-77.043,45.], Italiana, [88.,400.,0.11], Center, Upper, Clean, 0xe7d7be, 0xb79b79, "es", Italiana, Dissolve),
    p!("cusco", "Peru", [-13.532,-71.968,75.], Alfa, [66.,400.,0.005], Center, Upper, Press, 0xe4b16f, 0x7f4846, "es", Alfa, Print),
    p!("bogota", "Colombia", [4.711,-74.072,45.], Bricolage, [78.,650.,-0.025], Center, Source, Clean, 0xf0ce8e, 0xab7955, "es", Bricolage, Print),
    p!("cartagena", "Colombia", [10.392,-75.479,35.], FrauncesItalic, [73.,550.,-0.015], Stack, Source, SplitInk, 0xf4c4b5, 0xc5e0c2, "es", FrauncesItalic, Develop),
    p!("quito", "Ecuador", [-0.181,-78.468,50.], Cinzel, [70.,500.,0.065], Center, Upper, Edge, 0xf3dfb5, 0x947b52, "es", Cinzel, Dissolve),
    p!("la-paz", "Bolivia", [-16.5,-68.15,45.], Archivo, [73.,400.,0.005], Stack, Upper, Press, 0xeebf82, 0x845c78, "es", Archivo, Print),
    // Africa is deliberately several typographic vocabularies, not one continental motif.
    p!("dakar", "Senegal", [14.717,-17.467,55.], Alfa, [80.,400.,-0.015], Center, Upper, Press, 0xeac06e, 0x77435e, "fr", Alfa, Print),
    p!("saint-louis", "Senegal", [16.018,-16.49,40.], DMSerifItalic, [69.,400.,0.01], Right, Source, Clean, 0xeedbb9, 0x91adb5, "fr", DMSerifItalic, Dissolve),
    p!("accra", "Ghana", [5.603,-0.187,55.], Bricolage, [78.,800.,-0.045], Left, Upper, Edge, 0xf3c96c, 0x898752, "", Plex, Print),
    p!("lagos", "Nigeria", [6.524,3.379,65.], Unbounded, [72.,700.,-0.025], Center, Upper, Clean, 0xe4edd1, 0x7caa91, "", Plex, Print),
    p!("abidjan", "Ivory Coast", [5.36,-4.008,50.], Righteous, [71.,400.,0.015], Left, Source, Press, 0xe7dbb6, 0xc77c54, "fr", Righteous, Develop),
    p!("marrakech", "Morocco", [31.63,-7.981,75.], Italiana, [80.,400.,0.045], Center, Source, Glow, 0xeac28c, 0xb86f5c, "", Plex, Develop),
    p!("casablanca", "Morocco", [33.573,-7.59,50.], Josefin, [68.,500.,0.095], Wide, Upper, Edge, 0xf3e9d0, 0x779e9f, "", Plex, Print),
    p!("tunis", "Tunisia", [36.807,10.182,55.], Della, [76.,400.,0.065], Right, Upper, Clean, 0xe8e0c8, 0x79a9ba, "", Plex, Dissolve),
    p!("cairo", "Egypt", [30.044,31.236,75.], Cinzel, [84.,550.,0.09], Center, Upper, Edge, 0xf0d09b, 0x9c754c, "", Plex, Develop),
    p!("nairobi", "Kenya", [-1.292,36.822,55.], Archivo, [69.,400.,0.04], Left, Upper, Clean, 0xeee5bd, 0xb99673, "sw", Archivo, Print),
    p!("swahili-coast", "United Republic of Tanzania|Kenya", [-6.165,39.202,150.], CormorantItalic, [80.,600.,0.0], Center, Source, Clean, 0xe0ead8, 0x88b5b1, "sw", CormorantItalic, Dissolve),
    p!("addis-ababa", "Ethiopia", [9.032,38.747,70.], Della, [72.,400.,0.035], Center, Source, Edge, 0xefd998, 0x8c8860, "am", Ethiopic, Develop),
    p!("kigali", "Rwanda", [-1.944,30.059,45.], Josefin, [70.,400.,0.12], Wide, Upper, Clean, 0xd8e6ce, 0x91ac9b, "rw", Josefin, Dissolve),
    p!("cape-town", "South Africa", [-33.925,18.424,65.], DMSerifItalic, [82.,400.,0.0], Stack, Source, Clean, 0xc5e1d8, 0x89adbc, "", Plex, Dissolve),
    p!("johannesburg", "South Africa", [-26.204,28.047,60.], Bebas, [85.,400.,0.075], Left, Upper, Press, 0xefcb8c, 0x7f7370, "", Plex, Print),
    p!("namib", "Namibia", [-24.727,15.342,230.], Della, [76.,400.,0.16], Wide, Upper, Clean, 0xf0c19a, 0xb7826a, "", Plex, Dissolve),
    // Australia / Aotearoa: urban geometry, open landscape, coastal softness.
    p!("sydney", "Australia", [-33.869,151.209,60.], Archivo, [81.,400.,0.015], Center, Upper, Clean, 0xf1efdf, 0x9cc3d0, "", Plex, Print),
    p!("melbourne", "Australia", [-37.814,144.963,60.], Bricolage, [71.,550.,-0.035], Left, Source, Clean, 0xe8d6c8, 0xc39683, "", Plex, Develop),
    p!("uluru", "Australia", [-25.344,131.037,95.], Della, [85.,400.,0.11], Center, Upper, Press, 0xeac39a, 0x9f604b, "", Plex, Dissolve),
    p!("tropical-queensland", "Australia", [-16.918,145.778,170.], FrauncesItalic, [73.,500.,-0.01], Right, Source, Clean, 0xd4e6b7, 0x8eb9ac, "", Plex, Develop),
    p!("perth", "Australia", [-31.952,115.861,60.], Josefin, [80.,400.,0.08], Center, Upper, Glow, 0xf2dab9, 0x8eb8c6, "", Plex, Dissolve),
    p!("tasmania", "Australia", [-42.3,146.7,220.], CormorantItalic, [73.,500.,0.0], Intimate, Source, Clean, 0xd7dfd3, 0x9aaba2, "", Plex, Dissolve),
    p!("auckland", "New Zealand", [-36.849,174.763,60.], Unbounded, [57.,500.,-0.015], Left, Source, Clean, 0xd2e8e4, 0x90b8b9, "", Plex, Print),
    p!("southern-alps", "New Zealand", [-44.0,169.2,220.], Cormorant, [77.,500.,0.07], Center, Source, Edge, 0xe8eddf, 0x9eb5c4, "", Plex, Develop),
    // Europe: separate editorial, book, modernist, and travel-poster treatments.
    p!("london", "United Kingdom", [51.507,-0.128,55.], DMSerif, [78.,400.,0.0], Left, Source, Clean, 0xeee7d9, 0xb6b2a5, "", Plex, Print),
    p!("scottish-highlands", "United Kingdom", [57.2,-4.7,160.], CormorantItalic, [74.,600.,0.025], Intimate, Source, Glow, 0xdce2ce, 0x8caaa6, "", Plex, Dissolve),
    p!("paris", "France", [48.857,2.352,45.], BodoniItalic, [91.,500.,0.015], Center, Source, Clean, 0xf5e5d9, 0xc2a392, "fr", BodoniItalic, Develop),
    p!("provence", "France", [43.95,5.35,100.], FrauncesItalic, [68.,500.,0.005], Right, Source, Clean, 0xeddfa9, 0xac9ab4, "fr", FrauncesItalic, Dissolve),
    p!("lisbon", "Portugal", [38.722,-9.139,42.], FrauncesItalic, [81.,650.,-0.025], Center, Source, Press, 0xf0dbaa, 0x658ca9, "pt", FrauncesItalic, Develop),
    p!("porto", "Portugal", [41.158,-8.63,38.], DMSerif, [81.,400.,0.035], Left, Source, Edge, 0xe3e5ce, 0x7588a8, "pt", DMSerif, Print),
    p!("barcelona", "Spain", [41.387,2.17,40.], Bricolage, [75.,650.,-0.04], Center, Source, Press, 0xf3d1a3, 0xa97e65, "ca", Bricolage, Print),
    p!("andalusia", "Spain", [37.389,-5.984,100.], Della, [74.,400.,0.005], Right, Source, Edge, 0xf1dab0, 0xb27b63, "es", Della, Develop),
    p!("rome", "Italy", [41.903,12.496,50.], Cinzel, [85.,500.,0.045], Center, Upper, Clean, 0xead6b2, 0xb39470, "it", Cinzel, Develop),
    p!("venice", "Italy", [45.44,12.315,25.], CormorantItalic, [87.,600.,-0.015], Center, Source, Glow, 0xece1ca, 0x91bab8, "it", CormorantItalic, Dissolve),
    p!("milan", "Italy", [45.464,9.19,45.], Italiana, [86.,400.,0.04], Left, Upper, Clean, 0xf3eadf, 0xb4b0a8, "it", Italiana, Print),
    p!("berlin", "Germany", [52.52,13.405,50.], Unbounded, [71.,500.,-0.015], Left, Upper, Clean, 0xe5e7df, 0xa6aea6, "de", Unbounded, Print),
    p!("amsterdam", "Netherlands", [52.367,4.904,35.], Fraunces, [69.,600.,-0.02], Right, Source, Clean, 0xf0ddbd, 0xbd967b, "nl", Fraunces, Develop),
    p!("copenhagen", "Denmark", [55.676,12.568,35.], Josefin, [66.,400.,0.06], Left, Source, Clean, 0xe6dfd2, 0xb4bfb8, "da", Josefin, Dissolve),
    p!("reykjavik", "Iceland", [64.147,-21.943,75.], Josefin, [77.,300.,0.16], Wide, Upper, Glow, 0xd8eaf0, 0x9eaebf, "is", Josefin, Dissolve),
    p!("prague", "Czech Republic", [50.076,14.438,40.], Della, [80.,400.,0.02], Center, Source, Press, 0xe8cba2, 0x887367, "cs", Della, Develop),
    p!("vienna", "Austria", [48.208,16.374,40.], BodoniItalic, [86.,450.,0.0], Right, Source, Clean, 0xf2e5d3, 0xc1a786, "de", BodoniItalic, Develop),
    p!("athens", "Greece", [37.984,23.728,45.], Cinzel, [79.,400.,0.06], Center, Upper, Edge, 0xeee4c8, 0x97b4bd, "el", Plex, Dissolve),
    p!("istanbul", "Turkey", [41.008,28.978,55.], DMSerif, [76.,400.,0.02], Stack, Source, Glow, 0xedcca1, 0x82a6aa, "tr", DMSerif, Develop),
    // Asia: source-backed local scripts, each confined to its linguistic locality.
    p!("jaipur", "India", [26.912,75.787,100.], Yatra, [78.,400.,0.0], Center, Source, Press, 0xf0cd91, 0xb86b72, "hi", Yatra, Print),
    p!("mumbai", "India", [19.076,72.878,65.], Rozha, [85.,400.,0.0], Center, Source, Edge, 0xf3d392, 0x95645e, "mr", Rozha, Develop),
    p!("delhi", "India", [28.614,77.209,70.], Cinzel, [77.,500.,0.05], Wide, Upper, Clean, 0xead7b1, 0xaeb5a2, "hi", Rozha, Print),
    p!("kolkata", "India", [22.573,88.364,65.], DMSerif, [77.,400.,0.0], Left, Source, Clean, 0xeee0bb, 0xb28c74, "bn", Bangla, Develop),
    p!("chennai", "India", [13.083,80.271,65.], Fraunces, [72.,550.,-0.015], Center, Source, Press, 0xf0cb7d, 0xa66446, "ta", Tamil, Print),
    p!("hyderabad", "India", [17.385,78.487,65.], Della, [72.,400.,0.02], Right, Source, Glow, 0xe6d6b0, 0x8faaa4, "te", Telugu, Develop),
    p!("varanasi", "India", [25.318,82.974,60.], Cormorant, [79.,600.,0.025], Intimate, Source, Glow, 0xeccc99, 0xb88256, "hi", Yatra, Dissolve),
    p!("kathmandu", "Nepal", [27.717,85.324,55.], Yatra, [74.,400.,0.015], Center, Source, Clean, 0xe8d1b0, 0xb37468, "ne", Yatra, Develop),
    p!("dhaka", "Bangladesh", [23.81,90.412,55.], Bricolage, [76.,600.,-0.015], Left, Source, Clean, 0xdee1b5, 0x9cb699, "bn", Bangla, Print),
    p!("kyoto", "Japan", [35.012,135.768,28.], Cormorant, [74.,500.,0.09], Intimate, Source, Clean, 0xeee8d8, 0xba9890, "ja", Mincho, Develop),
    p!("tokyo", "Japan", [35.676,139.65,45.], Unbounded, [72.,600.,0.025], Left, Upper, Clean, 0xe1eaed, 0x99adc1, "ja", Cjk, Print),
    p!("osaka", "Japan", [34.694,135.502,30.], Righteous, [77.,400.,0.005], Center, Upper, Press, 0xf4d7a6, 0xbd6872, "ja", Cjk, Print),
    p!("hokkaido", "Japan", [43.1,142.4,240.], Josefin, [71.,300.,0.12], Wide, Upper, Clean, 0xe0ece6, 0x9fbdc0, "ja", Mincho, Dissolve),
    p!("seoul", "South Korea", [37.566,126.978,45.], Archivo, [78.,400.,0.02], Left, Upper, Edge, 0xf1e6d8, 0xb4899b, "ko", Hangul, Print),
    p!("busan", "South Korea", [35.18,129.076,35.], Josefin, [80.,500.,0.075], Center, Upper, Clean, 0xcde6e4, 0x96b8c7, "ko", Cjk, Dissolve),
    p!("beijing", "China", [39.904,116.407,55.], Cinzel, [76.,500.,0.08], Center, Upper, Edge, 0xf0dcac, 0xa37766, "zh", Cjk, Develop),
    p!("shanghai", "China", [31.23,121.474,55.], Josefin, [73.,500.,0.1], Wide, Upper, Glow, 0xe0e8d6, 0x96b8ae, "zh", Cjk, Print),
    p!("hong-kong", "Hong Kong S.A.R.", [22.319,114.169,35.], Bebas, [84.,400.,0.015], Stack, Upper, Edge, 0xe8e7d4, 0xa88a9c, "zh", Cjk, Print),
    p!("taipei", "Taiwan", [25.033,121.565,40.], Bricolage, [72.,600.,-0.015], Left, Source, Clean, 0xd8e8d4, 0xabb58b, "zh", Cjk, Develop),
    p!("bangkok", "Thailand", [13.756,100.502,60.], Della, [75.,400.,0.025], Center, Source, Glow, 0xf0d6a7, 0xc19469, "th", Thai, Develop),
    p!("hanoi", "Vietnam", [21.028,105.834,45.], Cormorant, [80.,600.,0.035], Intimate, Source, Clean, 0xe7dbbb, 0x9daf95, "vi", Cormorant, Dissolve),
    p!("singapore", "Singapore", [1.352,103.82,40.], Unbounded, [61.,450.,0.02], Left, Upper, Clean, 0xe4eadd, 0xa0bfb3, "", Plex, Print),
    p!("bali", "Indonesia", [-8.409,115.189,100.], FrauncesItalic, [80.,450.,0.015], Intimate, Source, Clean, 0xdde6c6, 0xa7b690, "id", FrauncesItalic, Dissolve),
    p!("dubai", "United Arab Emirates", [25.205,55.271,55.], Italiana, [89.,400.,0.11], Wide, Upper, Glow, 0xf3dfbf, 0xbcaaa0, "", Plex, Develop),
    p!("muscat", "Oman", [23.589,58.406,70.], Cormorant, [82.,500.,0.055], Center, Source, Clean, 0xecd5b0, 0xb6b798, "", Plex, Dissolve),
    // Country fallbacks remain restrained; city profiles never cover a whole nation.
    p!("swiss-alps", "Switzerland", [0.,0.,0.], CormorantItalic, [80.,500.,0.01], Center, Source, Clean, 0xe9e7d9, 0xb3ccd0, "", Plex, Dissolve),
    p!("italian-regions", "Italy", [0.,0.,0.], Cormorant, [72.,500.,0.015], Center, Source, Clean, 0xeaddc5, 0xb9a18a, "it", Cormorant, Develop),
    p!("french-regions", "France", [0.,0.,0.], DMSerifItalic, [69.,400.,0.01], Intimate, Source, Clean, 0xeee3d3, 0xbdb6aa, "fr", DMSerifItalic, Dissolve),
    p!("iberian-regions", "Spain|Portugal", [0.,0.,0.], Della, [70.,400.,0.025], Center, Source, Clean, 0xe9d6b4, 0xadb6aa, "", Plex, Dissolve),
    p!("nordic-regions", "Norway|Sweden|Finland|Iceland|Denmark", [0.,0.,0.], Josefin, [67.,300.,0.095], Wide, Source, Clean, 0xdde5df, 0xa4b8bb, "", Plex, Dissolve),
    p!("north-american-regions", "United States of America|Canada", [0.,0.,0.], DMSerif, [71.,400.,0.005], Center, Source, Clean, 0xece0c9, 0xadb9b2, "", Plex, Dissolve),
    p!("australian-regions", "Australia", [0.,0.,0.], Della, [73.,400.,0.07], Wide, Source, Clean, 0xe9d6b5, 0xb8aa93, "", Plex, Dissolve),
    p!("new-zealand-regions", "New Zealand", [0.,0.,0.], Cormorant, [75.,500.,0.035], Center, Source, Clean, 0xdce6d8, 0xa2b9b4, "", Plex, Dissolve),
    p!("brazilian-regions", "Brazil", [0.,0.,0.], Fraunces, [72.,500.,0.0], Center, Source, Clean, 0xdde3bc, 0xa5b598, "pt", Fraunces, Develop),
    p!("andean-regions", "Peru|Bolivia|Ecuador", [0.,0.,0.], Della, [71.,400.,0.05], Center, Source, Clean, 0xe6caa4, 0xb49a81, "", Plex, Dissolve),
    p!("indian-regions", "India", [0.,0.,0.], Della, [72.,400.,0.025], Center, Source, Clean, 0xead8bb, 0xbba589, "", Plex, Dissolve),
    p!("japanese-regions", "Japan", [0.,0.,0.], Cormorant, [70.,500.,0.055], Intimate, Source, Clean, 0xe8e5d8, 0xb9c5bb, "ja", Mincho, Dissolve),
    p!("korean-regions", "South Korea", [0.,0.,0.], Plex, [62.,400.,0.07], Center, Source, Clean, 0xe6e9df, 0xafc3c0, "ko", Cjk, Dissolve),
];

pub fn select(country: Option<&str>, lat: f64, lon: f64) -> &'static Profile {
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90. || lon.abs() > 180. {
        return &FALLBACK;
    }
    let Some(country) = country else {
        return &FALLBACK;
    };
    let mut broad = &FALLBACK;
    let mut local: Option<(&Profile, f64)> = None;
    for profile in PROFILES {
        if !profile.countries.split('|').any(|c| c == country) {
            continue;
        }
        let [plat, plon, radius] = profile.center;
        if radius == 0. {
            broad = profile;
            continue;
        }
        let dlat = (lat - plat).to_radians();
        let dlon = (lon - plon).to_radians();
        let hav = (dlat / 2.).sin().powi(2)
            + lat.to_radians().cos() * plat.to_radians().cos() * (dlon / 2.).sin().powi(2);
        let km = 12_742. * hav.clamp(0., 1.).sqrt().asin();
        if km <= radius && local.is_none_or(|(_, previous)| km / radius < previous) {
            local = Some((profile, km / radius));
        }
    }
    local.map_or(broad, |(profile, _)| profile)
}
