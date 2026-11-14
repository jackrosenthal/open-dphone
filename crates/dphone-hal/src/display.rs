//! Displays and backlight.

use crate::{Model, Result, Size};

/// Which panel.  Only the D70 has [`Screen::Side`] (113x357 rapid-dial LCD).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    Main,
    Side,
}

/// Pixel layout of a framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 32 bits per pixel, little-endian `B, G, R, x` in memory (DRM
    /// `XR24`).  Color models.
    Xrgb8888,
    /// 8 bits per pixel gray, 0 = black, 255 = white (DRM `R8  `).  The
    /// panels show four levels; the driver uses the top two bits.
    R8,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Xrgb8888 => 4,
            PixelFormat::R8 => 1,
        }
    }
}

/// Geometry of one screen's framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub width: u16,
    pub height: u16,
    /// Bytes per row (may exceed `width * bytes_per_pixel`).
    pub stride: usize,
    pub format: PixelFormat,
}

impl FrameInfo {
    /// The natural layout for a model's screen.
    pub fn for_model(model: Model, screen: Screen) -> Option<FrameInfo> {
        let size = match screen {
            Screen::Main => model.main_screen(),
            Screen::Side => model.side_screen()?,
        };
        let format = if model.is_color() { PixelFormat::Xrgb8888 } else { PixelFormat::R8 };
        Some(FrameInfo {
            width: size.width,
            height: size.height,
            stride: size.width as usize * format.bytes_per_pixel(),
            format,
        })
    }
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
    /// Total bytes of the framebuffer.
    pub const fn len(&self) -> usize {
        self.stride * self.height as usize
    }
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A rectangle in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Rect { x, y, width, height }
    }
    /// Clips to a `width` x `height` screen.
    pub fn clip(self, width: u16, height: u16) -> Rect {
        let x = self.x.min(width);
        let y = self.y.min(height);
        Rect {
            x,
            y,
            width: self.width.min(width - x),
            height: self.height.min(height - y),
        }
    }
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A copy of a framebuffer (screen mirror, screenshots, tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub info: FrameInfo,
    pub pixels: Vec<u8>,
}

/// The panels.  Drawing is: get [`Display::frame`], write pixels, then
/// [`Display::flush`] with the damaged rectangles.  Nothing is guaranteed
/// visible before the flush (the SPI panels are pushed on flush; the color
/// panel scans out of the buffer directly).
pub trait Display: Send {
    /// The screens this model has.
    fn screens(&self) -> Vec<Screen>;
    /// Geometry, or `None` if this model lacks that screen.
    fn info(&self, screen: Screen) -> Option<FrameInfo>;
    /// The pixel memory to draw into, [`FrameInfo::len`] bytes.
    fn frame(&mut self, screen: Screen) -> Result<&mut [u8]>;
    /// Makes the damaged areas visible.  An empty slice means the whole
    /// screen.
    fn flush(&mut self, screen: Screen, damage: &[Rect]) -> Result<()>;
    /// Copies out what was last flushed (or the current buffer).
    fn capture(&self, screen: Screen) -> Result<Frame>;
    /// Turns the panel itself on or off (DPMS).  Backlight is separate.
    fn set_power(&mut self, screen: Screen, on: bool) -> Result<()>;
}

/// Backlight brightness and (gray panels) contrast.
pub trait Backlight: Send {
    /// Highest brightness level (32 color, 31 gray); 0 is off.
    fn max_brightness(&self, screen: Screen) -> u8;
    fn brightness(&self, screen: Screen) -> Result<u8>;
    fn set_brightness(&mut self, screen: Screen, level: u8) -> Result<()>;
    /// Highest contrast level: 100 (contrast is a percentage, as in the
    /// stock settings), or 0 when the panel has no contrast control.
    fn max_contrast(&self, screen: Screen) -> u8;
    fn contrast(&self, screen: Screen) -> Result<u8>;
    fn set_contrast(&mut self, screen: Screen, level: u8) -> Result<()>;
}
