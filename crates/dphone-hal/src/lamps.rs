//! Lamps (LEDs) and the userspace blinker.

use crate::{Blink, LampColor, LampState, Model, Result};

/// A lamp, named by function.  Indices are 0-based from the top
/// (`Lamp::Line(0)` belongs to `Key::Line(1)`).
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
    /// The lamps in a model's lamp table.  (A D60 has fewer line lamps
    /// fitted than its table lists; setting an unfitted lamp is harmless.)
    pub fn for_model(model: Model) -> Vec<Lamp> {
        let mut v: Vec<Lamp> = (0..model.line_lamps()).map(Lamp::Line).collect();
        v.extend((0..model.blf_lamps()).map(Lamp::Blf));
        v.extend([Lamp::Message, Lamp::Mute, Lamp::Headset, Lamp::Speaker]);
        v
    }
}

/// The lamps.  On/off per color only; blinking is done by [`Blinker`].
pub trait Lamps: Send {
    /// Lamps that exist on this phone.
    fn lamps(&self) -> Vec<Lamp>;
    /// Sets a lamp's color now.  Unknown lamps are ignored.
    fn set(&mut self, lamp: Lamp, color: LampColor) -> Result<()>;
}

/// Blink timing in userspace: holds the wanted [`LampState`] of each lamp
/// and drives a [`Lamps`] device from a periodic [`Blinker::tick`].
///
/// All blinking lamps share one phase, so they flash together.
#[derive(Default)]
pub struct Blinker {
    wanted: Vec<(Lamp, LampState)>,
    shown: Vec<(Lamp, LampColor)>,
}

/// The longest interval between ticks that still renders every pattern.
pub const BLINK_TICK_MS: u64 = 125;

impl Blinker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets what a lamp should show.  Takes effect on the next tick.
    pub fn set(&mut self, lamp: Lamp, state: LampState) {
        match self.wanted.iter_mut().find(|(l, _)| *l == lamp) {
            Some(e) => e.1 = state,
            None => self.wanted.push((lamp, state)),
        }
    }

    /// What a lamp was last asked to show.
    pub fn get(&self, lamp: Lamp) -> LampState {
        self.wanted.iter().find(|(l, _)| *l == lamp).map(|e| e.1).unwrap_or(LampState::OFF)
    }

    /// True when any lamp blinks, i.e., ticks are needed.
    pub fn needs_ticks(&self) -> bool {
        self.wanted.iter().any(|(_, s)| s.blink != Blink::Solid && s.color != LampColor::Off)
    }

    /// Whether a pattern is in its "on" part at `now_ms`.
    pub fn phase_on(blink: Blink, now_ms: u64) -> bool {
        match blink {
            Blink::Solid => true,
            // 1 Hz, half on.
            Blink::Slow => now_ms % 1000 < 500,
            // 4 Hz, half on.
            Blink::Fast => now_ms % 250 < 125,
            // 125 ms flash every 2 s.
            Blink::Wink => now_ms % 2000 < 125,
        }
    }

    /// Applies the current phase to the device, writing only lamps whose
    /// color changed.  Call every [`BLINK_TICK_MS`] while
    /// [`Blinker::needs_ticks`], and once after any [`Blinker::set`].
    pub fn tick(&mut self, now_ms: u64, dev: &mut dyn Lamps) -> Result<()> {
        let mut first_err = None;
        for (lamp, state) in &self.wanted {
            let color = if Self::phase_on(state.blink, now_ms) { state.color } else { LampColor::Off };
            let shown = self.shown.iter_mut().find(|(l, _)| l == lamp);
            if shown.as_ref().is_some_and(|s| s.1 == color) {
                continue;
            }
            match dev.set(*lamp, color) {
                Ok(()) => match shown {
                    Some(s) => s.1 = color,
                    None => self.shown.push((*lamp, color)),
                },
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
