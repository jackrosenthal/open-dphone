//! The SDK for Open DPhone apps (the README tells how to build, package,
//! and install one).
//!
//! An app is a `cdylib` built for `wasm32-unknown-unknown` that implements
//! [`App`] and registers it with [`export_app!`]:
//!
//! ```ignore
//! use dphone_app_sdk::prelude::*;
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl App for Hello {
//!     fn start(&mut self) {
//!         let t = ui::create(WidgetKind::Text, ROOT, Rect::new(0, 0, 200, 20));
//!         ui::set_text(t, "Hello");
//!     }
//!     fn handle(&mut self, _e: Event) {}
//! }
//!
//! dphone_app_sdk::export_app!(Hello);
//! ```
//!
//! The interface is defined once, in `wit/app.wit` in this crate; the
//! modules re-exported here ([`ui`], [`pbx`], [`calls`], ...) are generated
//! from it.  An app is given the interfaces its component imports, and no
//! others.

#![allow(clippy::too_many_arguments)]

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "app",
        pub_export_macro: true,
        default_bindings_module: "dphone_app_sdk::bindings",
        generate_all,
    });
}

pub mod json;
pub mod manifest;
pub mod nav;
pub mod xml;

pub use bindings::dphone::app::types::{
    AccountInfo, Align, CallInfo, CallState, Error, Event, Font, Header, HttpResponse, InputMode,
    ListItem, Metrics, Mwi, PbxOutcome, Presence, Push, Rect, Size, Skin, Softkey,
    WidgetEvent, WidgetKind,
};
pub use bindings::dphone::app::{
    audio, calls, canvas, config, diagnostics, events, hotdesk, http, input, l10n, lamps, pbx,
    presence, runtime, storage, timers, ui,
};

/// A widget handle.
pub type Widget = u32;
/// The app window's root widget.
pub const ROOT: Widget = 0;

/// What an app implements.
pub trait App: Default + 'static {
    /// Called once after the app is loaded.
    fn start(&mut self);
    /// Called for every event.
    fn handle(&mut self, event: Event);
}

impl Rect {
    pub const fn new(x: i16, y: i16, width: u16, height: u16) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
}

impl ListItem {
    pub fn new(text: impl Into<String>) -> ListItem {
        ListItem {
            text: text.into(),
            detail: String::new(),
            icon: String::new(),
        }
    }
    pub fn with_detail(text: impl Into<String>, detail: impl Into<String>) -> ListItem {
        ListItem {
            text: text.into(),
            detail: detail.into(),
            icon: String::new(),
        }
    }
}

impl Softkey {
    pub fn new(id: u32, label: impl Into<String>) -> Softkey {
        Softkey {
            id,
            label: label.into(),
        }
    }
}

/// Log helpers.
pub fn log_error(msg: &str) {
    runtime::log(0, msg);
}
pub fn log_warn(msg: &str) {
    runtime::log(1, msg);
}
pub fn log_info(msg: &str) {
    runtime::log(2, msg);
}
pub fn log_debug(msg: &str) {
    runtime::log(3, msg);
}

/// The full content area as a rectangle at the origin.
pub fn content_rect() -> Rect {
    let m = ui::get_metrics();
    Rect::new(0, 0, m.width, m.height)
}

/// Row height of the skin.
pub fn row_height() -> u16 {
    ui::get_metrics().row_height
}

/// Clears the window, draws a title line, and returns the rectangle left
/// below it.  The window claims `cancel`, so the app sees it as
/// [`Event::Key`] and decides between "back" and leaving.
pub fn screen(title: &str) -> Rect {
    ui::clear(ROOT);
    ui::claim_keys(ROOT, &["cancel".to_string()]);
    let m = ui::get_metrics();
    let t = ui::create(
        WidgetKind::Text,
        ROOT,
        Rect::new(0, 0, m.width, m.title_height),
    );
    ui::set_font(t, Font::Title);
    ui::set_text(t, title);
    Rect::new(
        0,
        m.title_height as i16,
        m.width,
        m.height.saturating_sub(m.title_height),
    )
}

/// The value of a storage key as UTF-8 text.
pub fn read_string(key: &str) -> Option<String> {
    storage::get(key).and_then(|b| String::from_utf8(b).ok())
}

pub mod prelude {
    pub use crate::json::Json;
    pub use crate::manifest::AppKind;
    pub use crate::nav::{Form, Message};
    pub use crate::{
        App, Event, ListItem, PbxOutcome, ROOT, Rect, Skin, Softkey, Widget, WidgetEvent,
        WidgetKind, audio, calls, config, content_rect, events, http, l10n, pbx, presence,
        row_height, runtime, screen, storage, timers, ui,
    };
}

/// Registers the app with the phone: the type that implements [`App`], and
/// the app's manifest (the fields of [`manifest::Manifest`]), which goes
/// into the app's WebAssembly file.
///
/// ```ignore
/// dphone_app_sdk::export_app! {
///     app: Hello,
///     name: "hello",
///     display_name: "Hello",
///     description: "Says hello",
/// }
/// ```
///
/// `app` comes first; `name` and `display_name` must be given; the other
/// fields may be: `id`, `display_name_in`, `description`, `description_in`,
/// `kind`, `has_idle`, `hidden`, `icon`, `icon_for`, `phone_icon`, and
/// `memory_kib`.  A
/// field the phone would refuse does not compile.
#[macro_export]
macro_rules! export_app {
    (app: $ty:ty, $($field:ident : $value:expr),+ $(,)?) => {
        const _: () = {
            const MANIFEST: $crate::manifest::Manifest = $crate::manifest::Manifest {
                $($field: $value,)+
                ..$crate::manifest::Manifest::EMPTY
            }
            .checked();
            const LEN: usize = MANIFEST.encoded_len();
            // A custom section of the WebAssembly module.
            #[cfg_attr(target_family = "wasm", unsafe(link_section = "dphone-app"))]
            #[used]
            static SECTION: [u8; LEN] = MANIFEST.encode::<LEN>();
        };

        #[doc(hidden)]
        mod __dphone_app_export {
            use super::*;
            use core::cell::RefCell;
            struct Cell(RefCell<Option<$ty>>);
            // SAFETY: a WebAssembly app instance is single-threaded.
            unsafe impl Sync for Cell {}
            static APP: Cell = Cell(RefCell::new(None));
            pub struct Shim;
            impl $crate::bindings::exports::dphone::app::guest::Guest for Shim {
                fn start() {
                    let mut slot = APP.0.borrow_mut();
                    let app = slot.insert(<$ty as Default>::default());
                    $crate::App::start(app);
                }
                fn handle(e: $crate::Event) {
                    if let Some(app) = APP.0.borrow_mut().as_mut() {
                        $crate::App::handle(app, e);
                    }
                }
            }
            $crate::bindings::export!(Shim with_types_in $crate::bindings);
        }
    };
    ($ty:ty) => {
        compile_error!(
            "export_app! takes the app's manifest too: export_app! { app: Type, name: \"name\", display_name: \"Name\" }"
        );
    };
}

/// Sends a PBX request with `parameters` (an object), adding the app
/// account's `account_id` when the provisioned configuration has one and
/// the parameters do not.  The reply arrives as [`Event::Pbx`].
pub fn pbx_request(method: &str, parameters: json::Json) -> u32 {
    let mut members = match parameters {
        json::Json::Obj(m) => m,
        _ => Vec::new(),
    };
    if !members.iter().any(|(k, _)| k == "account_id")
        && let Some(id) = config::app_setting("account_id")
        && !id.is_empty()
    {
        members.push(("account_id".to_string(), json::Json::Str(id)));
    }
    pbx::request(method, &json::Json::Obj(members).to_string())
}

/// Text for a failed PBX request, for an error screen.
pub fn pbx_error_text(outcome: &PbxOutcome) -> String {
    match outcome {
        PbxOutcome::Ok(_) => String::new(),
        PbxOutcome::Errors(e) => {
            let v = json::Json::parse(e).unwrap_or(json::Json::Null);
            let first = v.get("error");
            let first = first.items().first().copied().cloned().unwrap_or(v.clone());
            let msg = first.get("message").text();
            if msg.is_empty() {
                "The server reported an error.".to_string()
            } else {
                msg
            }
        }
        PbxOutcome::Transport(t) => format!("Could not reach the server.\n{t}"),
    }
}
