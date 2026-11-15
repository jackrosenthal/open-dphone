//! How to play, in a few pages: each a small grid that shows a rule, drawn
//! as the board draws it, and a sentence or two.  The words are this
//! project's own.

use crate::Grid;

pub struct Page {
    /// The grid of the picture, as [`Grid::parse`] reads it.
    pub picture: &'static str,
    pub text: &'static str,
}

impl Page {
    pub fn grid(&self) -> Grid {
        Grid::parse(self.picture).expect("a grid")
    }
}

/// The pictures are five cells by three, so that a page fits the smallest
/// display beside its text.
pub const PAGES: [Page; 4] = [
    Page {
        picture: ".....\n.L.#.\n.....",
        text: "Light every white cell.  A light shines along its row and column until a black cell.",
    },
    Page {
        picture: ".....\nL..L.\n.....",
        text: "No light may shine on another light.",
    },
    Page {
        picture: ".L#L.\n.1.1L\n.....",
        text: "A numbered cell needs exactly that many lights next to it.",
    },
    Page {
        picture: ".x0x.\n..x..\n.....",
        text: "A square is your note that a cell gets no light, as next to a 0.",
    },
];
