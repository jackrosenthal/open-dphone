//! The built-in wallpapers of the color models, by name: what the
//! configuration can name and the web UI lists.  The pictures and the
//! palettes are the UI's (`dphone_ui::wallpaper`, whose tests hold this
//! list to its own).

/// One built-in wallpaper: its name in the configuration, the name shown,
/// and the family it is listed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallpaperName {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
}

const fn w(id: &'static str, name: &'static str, family: &'static str) -> WallpaperName {
    WallpaperName { id, name, family }
}

/// Every built-in wallpaper, grouped by family.  The first is the default.
pub const WALLPAPERS: [WallpaperName; 42] = [
    w("harbor", "Harbor", "Facets"),
    w("evergreen", "Evergreen", "Facets"),
    w("merlot", "Merlot", "Facets"),
    w("graphite", "Graphite", "Facets"),
    w("amethyst", "Amethyst", "Facets"),
    w("ember", "Ember", "Facets"),
    w("lagoon", "Lagoon", "Facets"),
    w("dusk", "Dusk", "Gradients"),
    w("aurora", "Aurora", "Gradients"),
    w("deep-sea", "Deep Sea", "Gradients"),
    w("slate", "Slate", "Gradients"),
    w("nebula", "Nebula", "Gradients"),
    w("sunrise", "Sunrise", "Gradients"),
    w("mist", "Mist", "Gradients"),
    w("bokeh", "Bokeh", "Bubbles"),
    w("fizz", "Fizz", "Bubbles"),
    w("lanterns", "Lanterns", "Bubbles"),
    w("orbit", "Orbit", "Bubbles"),
    w("pebbles", "Pebbles", "Bubbles"),
    w("slant", "Slant", "Modern"),
    w("regatta", "Regatta", "Modern"),
    w("prism", "Prism", "Modern"),
    w("glacier", "Glacier", "Modern"),
    w("tides", "Tides", "Modern"),
    w("dunes", "Dunes", "Modern"),
    w("summit", "Summit", "Modern"),
    w("pinpoint", "Pinpoint", "Modern"),
    w("drift", "Drift", "Modern"),
    w("eclipse", "Eclipse", "Modern"),
    w("houndstooth", "Houndstooth", "Classic"),
    w("herringbone", "Herringbone", "Classic"),
    w("chevron", "Chevron", "Classic"),
    w("tartan", "Tartan", "Classic"),
    w("argyle", "Argyle", "Classic"),
    w("polka", "Polka", "Classic"),
    w("pinstripe", "Pinstripe", "Classic"),
    w("hot-dog-stand", "Hot Dog Stand", "Classic"),
    w("black", "Black", "Solids"),
    w("charcoal", "Charcoal", "Solids"),
    w("navy", "Navy", "Solids"),
    w("forest", "Forest", "Solids"),
    w("stone", "Stone", "Solids"),
];
