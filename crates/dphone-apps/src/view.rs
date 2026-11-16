//! The window an app draws into, as the app host sees it.  The UI toolkit
//! (`dphone-ui`'s `appview`) is adapted to these traits by the integrator;
//! the semantics follow that module: widgets are handles within one window,
//! `None` is the window itself (the app content area), keys reach the app
//! only when claimed, and softkeys belong to widgets.

pub use crate::host::bindings::dphone::app::canvas::{
    Bitmap, Color, Op as DrawOp, Role as ColorRole,
};
pub use crate::host::bindings::dphone::app::types::{
    Align, Font, InputMode, ListItem, Metrics, Rect, Size, Skin, Softkey, WidgetKind,
};
use dphone_state::{Key, LampColor, Model};
use std::sync::Arc;

/// A lamp of the phone, as apps name it.  Line and rapid-dial lamps count
/// from 0 at the top (`line1` is `Lamp::Line(0)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lamp {
    Line(u8),
    Blf(u8),
    /// Message waiting.
    Message,
    Mute,
    Headset,
    Speaker,
}

impl Lamp {
    /// The lamps of a model.
    pub fn for_model(model: Model) -> Vec<Lamp> {
        // The lamp table of a model may list more line lamps than the
        // model has line keys (the D60 and D62 have two of the D65's six).
        let lines = model.line_lamps().min(model.line_keys());
        let mut v: Vec<Lamp> = (0..lines).map(Lamp::Line).collect();
        v.extend((0..model.blf_lamps()).map(Lamp::Blf));
        v.extend([Lamp::Message, Lamp::Mute, Lamp::Headset, Lamp::Speaker]);
        v
    }

    /// The name of the lamp in the app interface: that of its key, where
    /// it is in one.
    pub fn name(self) -> String {
        match self {
            Lamp::Line(i) => format!("line{}", u16::from(i) + 1),
            Lamp::Blf(i) => format!("blf{}", u16::from(i) + 1),
            Lamp::Message => "message".into(),
            Lamp::Mute => "mute".into(),
            Lamp::Headset => "hdst".into(),
            Lamp::Speaker => "spkr".into(),
        }
    }
}

/// Which of an app's two possible windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowKind {
    /// The full app screen.
    App,
    /// The app's area on the idle screen (`hasIdle`).
    Idle,
}

/// Something the user did in a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    /// The window became the top screen.
    Foreground,
    /// The window stopped being the top screen.
    Background,
    /// A claimed key was pressed, or any key while the window captures
    /// input.
    Key(Key),
    /// The handset was lifted (`true`) or put back while the window
    /// captures input.
    Hook(bool),
    Softkey(u32),
    Highlighted {
        widget: u32,
        row: u32,
    },
    Activated {
        widget: u32,
        row: u32,
    },
    Changed {
        widget: u32,
        value: String,
    },
    Submitted {
        widget: u32,
        value: String,
    },
    /// Cancel was pressed and nothing claimed it.
    CloseRequested,
}

/// One window of one app.  Calls on dead handles are ignored.
pub trait AppWindow: Send {
    fn metrics(&self) -> Metrics;
    fn create(&mut self, kind: WidgetKind, parent: Option<u32>, bounds: Rect) -> u32;
    fn destroy(&mut self, w: u32);
    /// Destroys the children of `w` (`None`: everything in the window, and
    /// resets the window's softkeys and key claims).
    fn clear(&mut self, w: Option<u32>);
    fn set_bounds(&mut self, w: u32, bounds: Rect);
    /// Hiding the focused widget, or one the focused widget is in, leaves
    /// no widget focused.
    fn set_visible(&mut self, w: u32, visible: bool);
    fn set_focus(&mut self, w: Option<u32>);
    fn set_text(&mut self, w: u32, text: &str);
    fn set_align(&mut self, w: u32, align: Align);
    fn set_font(&mut self, w: u32, font: Font);
    fn set_wrap(&mut self, w: u32, wrap: bool);
    /// Draws the widget in the skin's selection colors.
    fn set_highlight(&mut self, w: u32, on: bool);
    fn set_items(&mut self, w: u32, items: &[ListItem]);
    fn set_selected(&mut self, w: u32, index: u32);
    /// The highlighted row of a list that has rows.
    fn selected(&self, w: u32) -> Option<u32>;
    fn set_value(&mut self, w: u32, value: &str);
    fn value(&self, w: u32) -> String;
    fn set_input_mode(&mut self, w: u32, mode: InputMode);
    fn set_max_length(&mut self, w: u32, max: u32);
    /// Shows the picture defined as `name`, else the phone's icon of that
    /// name.
    fn set_image(&mut self, w: u32, name: &str);
    /// Makes a picture the app defined (`file` is a PNG file's content)
    /// known as `name` to [`AppWindow::set_image`] and to list icons,
    /// in place of an earlier one of that name.  Returns whether it is a
    /// usable picture.
    fn define_image(&mut self, _name: &str, _file: &[u8]) -> bool {
        false
    }
    /// Gives the window the whole display, with no softkey bar, or the
    /// usual area again; [`AppWindow::metrics`] follows.
    fn set_full_screen(&mut self, _on: bool) {}
    /// Gives the window every key and hookswitch change while it is on
    /// screen, or the usual key handling again.
    fn capture_input(&mut self, _on: bool) {}
    /// Shows a color on a lamp, in place of the phone's, while the window
    /// is on screen.
    fn set_lamp(&mut self, _lamp: Lamp, _color: LampColor) {}
    /// Gives every lamp back to the phone.
    fn release_lamps(&mut self) {}
    /// A color of the skin (capability `canvas`).
    fn theme_color(&self, _role: ColorRole) -> Color {
        Color { r: 0, g: 0, b: 0 }
    }
    /// The width and height of what a text paints in a font of the skin.
    fn text_size(&self, _font: Font, _text: &str) -> (u16, u16) {
        (0, 0)
    }
    /// Creates a canvas widget.
    fn create_canvas(&mut self, _parent: Option<u32>, _bounds: Rect) -> u32 {
        0
    }
    /// Replaces what a canvas widget shows, and returns whether `w` is
    /// one.  The host has checked the limits of the interface.
    fn draw(&mut self, _w: u32, _ops: Vec<DrawOp>) -> bool {
        false
    }
    /// With `true`, what is changed in the window from now on is not
    /// shown until this is called with `false`: the app's handler runs,
    /// and the changes the handler makes appear together.
    fn hold_changes(&mut self, _on: bool) {}
    fn set_softkeys(&mut self, w: Option<u32>, keys: &[Softkey]);
    fn claim_keys(&mut self, w: Option<u32>, keys: &[Key]);
    /// Next pending user event, if any.
    fn poll_event(&mut self) -> Option<WindowEvent>;
}

/// The screen manager side: creates windows and moves them on and off the
/// navigation stack.
pub trait Shell: Send {
    /// Creates a window for `app`.  It is not on screen until [`Shell::show`].
    /// `wake` must be called (from any thread) whenever the window has a
    /// new event to poll.
    fn open(
        &mut self,
        app: &str,
        kind: WindowKind,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Box<dyn AppWindow>;
    /// Puts the app's window on top of the navigation stack.
    fn show(&mut self, app: &str);
    /// Takes the app's window off the screen (the previous screen shows).
    fn hide(&mut self, app: &str);
    /// The window is gone for good.
    fn close(&mut self, app: &str, kind: WindowKind);
    /// Size of the idle-screen area offered to apps.
    fn idle_size(&self) -> Size;
}
