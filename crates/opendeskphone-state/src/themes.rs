//! The built-in themes of the color models, by name: what the
//! configuration can name and the web UI lists.  A theme is a wallpaper
//! and a palette for every screen; both are the UI's
//! (`opendeskphone_ui::wallpaper`, whose tests hold this list to its own).

/// The theme when none is chosen.
pub const DEFAULT_THEME: &str = "tides";

/// One built-in theme: its name in the configuration, the name shown,
/// and the family it is listed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeName {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
}

const fn w(id: &'static str, name: &'static str, family: &'static str) -> ThemeName {
    ThemeName { id, name, family }
}

/// Every built-in theme, grouped by family.  The default is
/// [`DEFAULT_THEME`].
pub const THEMES: [ThemeName; 42] = [
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
    w("regatta", "Regatta", "Gradients"),
    w("bokeh", "Bokeh", "Bubbles"),
    w("fizz", "Fizz", "Bubbles"),
    w("lanterns", "Lanterns", "Bubbles"),
    w("orbit", "Orbit", "Bubbles"),
    w("pebbles", "Pebbles", "Bubbles"),
    w("slant", "Slant", "Modern"),
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
