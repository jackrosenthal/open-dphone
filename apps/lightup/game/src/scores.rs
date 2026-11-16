//! The best times: a table for each kind of puzzle (a size with a
//! difficulty), which the phones of a PBX share, and the times a phone
//! has yet to write there.
//!
//! A table is the value of one key of the app's store on the PBX,
//! [`Kind::key`] (e.g., `best/7x7-easy`).  It is text, a line for each
//! row, the best first: the time in milliseconds, the player's id at
//! the PBX, and the player's name, with a tab between them.  A player
//! is in a table once, with the best of their times, and a table has at
//! most [`ROWS`] rows.
//!
//! The times to write are text as well, a line for each kind of puzzle
//! that has one: the kind (`7x7-easy`), a space, and the milliseconds.

use crate::generate::{Difficulty, SIZES};

/// The most rows a table has.
pub const ROWS: usize = 10;
/// The most characters of a player's name a table keeps.
pub const NAME_CHARS: usize = 32;

/// A kind of puzzle: a size with a difficulty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub side: usize,
    pub difficulty: Difficulty,
}

impl Kind {
    /// How many kinds there are.
    pub const COUNT: usize = SIZES.len() * Difficulty::ALL.len();

    pub fn new(side: usize, difficulty: Difficulty) -> Kind {
        Kind { side, difficulty }
    }

    /// Every kind, in the order of the list of new puzzles: the sizes,
    /// and for each its difficulties.
    pub fn all() -> impl Iterator<Item = Kind> {
        SIZES
            .into_iter()
            .flat_map(|side| Difficulty::ALL.map(|difficulty| Kind { side, difficulty }))
    }

    /// Where the kind is in [`Kind::all`] (a size that is not one of
    /// [`SIZES`] counts as the first).
    pub fn index(self) -> usize {
        let size = SIZES.iter().position(|s| *s == self.side).unwrap_or(0);
        size * Difficulty::ALL.len() + self.difficulty as usize
    }

    pub fn from_index(index: usize) -> Kind {
        let n = Difficulty::ALL.len();
        Kind {
            side: SIZES[index / n % SIZES.len()],
            difficulty: Difficulty::ALL[index % n],
        }
    }

    /// The kind with the next size (after the largest, the smallest) and
    /// the same difficulty.
    pub fn next_size(self) -> Kind {
        Kind::from_index(self.index() + Difficulty::ALL.len())
    }

    /// The kind with the next difficulty (after the hardest, the
    /// easiest) and the same size.
    pub fn next_difficulty(self) -> Kind {
        let n = Difficulty::ALL.len();
        Kind {
            side: self.side,
            difficulty: Difficulty::ALL[(self.difficulty as usize + 1) % n],
        }
    }

    /// The kind `steps` further in [`Kind::all`], around the ends.
    pub fn step(self, steps: isize) -> Kind {
        let count = Kind::COUNT as isize;
        Kind::from_index((self.index() as isize + steps).rem_euclid(count) as usize)
    }

    /// As the screens name it: `7x7 Easy`.
    pub fn title(self) -> String {
        format!("{0}x{0} {1}", self.side, self.difficulty.name())
    }

    /// As the keys and the files name it: `7x7-easy`.
    pub fn name(self) -> String {
        format!("{0}x{0}-{1}", self.side, self.difficulty.name().to_lowercase())
    }

    pub fn named(name: &str) -> Option<Kind> {
        Kind::all().find(|k| k.name() == name)
    }

    /// The key of the kind's table in the app's store on the PBX.
    pub fn key(self) -> String {
        format!("best/{}", self.name())
    }

    /// The kind whose table a key of the store holds.
    pub fn of_key(key: &str) -> Option<Kind> {
        Kind::named(key.strip_prefix("best/")?)
    }
}

/// A time as the screens show it: `3:05`, or `1:02:03`.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A row of a table: a player and the best of their times.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Milliseconds.
    pub ms: u64,
    /// Identifies the player at the PBX.
    pub id: String,
    /// The player's name, to show.
    pub name: String,
}

/// The best times of a kind of puzzle, the best first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    rows: Vec<Row>,
}

/// A name or an id as a table keeps it: on one line and without tabs,
/// and no longer than `chars`.
fn field(text: &str, chars: usize) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clean.trim().chars().take(chars).collect()
}

impl Table {
    /// The table a value of the store holds.  What is not a row is left
    /// out, and so is what a table cannot hold: a second row of a
    /// player, and rows beyond [`ROWS`].
    pub fn parse(value: &[u8]) -> Table {
        let mut table = Table::default();
        for line in String::from_utf8_lossy(value).lines() {
            let mut fields = line.split('\t');
            let (Some(ms), Some(id), Some(name)) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if let Ok(ms) = ms.parse() {
                table.add(ms, id, name);
            }
        }
        table
    }

    /// The table as a value of the store.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = String::new();
        for row in &self.rows {
            text.push_str(&format!("{}\t{}\t{}\n", row.ms, row.id, row.name));
        }
        text.into_bytes()
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Where a player's row is.
    pub fn place(&self, id: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.id == id)
    }

    /// Puts a time of a player in the table: after the times that are
    /// as good (who was first stays first), and in place of the player's
    /// row if the table has one.  False if the table is as it was: the
    /// player has a time as good in it, or the time is not among the
    /// best [`ROWS`].
    pub fn add(&mut self, ms: u64, id: &str, name: &str) -> bool {
        let id = field(id, usize::MAX);
        let at = self.rows.iter().take_while(|r| r.ms <= ms).count();
        match self.place(&id) {
            Some(had) if had < at => return false,
            Some(had) => {
                self.rows.remove(had);
            }
            None if at >= ROWS => return false,
            None => {}
        }
        let name = field(name, NAME_CHARS);
        self.rows.insert(at, Row { ms, id, name });
        self.rows.truncate(ROWS);
        true
    }
}

/// The times a phone has yet to write to the tables: for each kind of
/// puzzle the best of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending {
    times: Vec<(Kind, u64)>,
}

impl Pending {
    /// What a file holds; a line that is not a time is left out.
    pub fn parse(text: &str) -> Pending {
        let mut pending = Pending::default();
        for line in text.lines() {
            if let Some((kind, ms)) = line.split_once(' ')
                && let (Some(kind), Ok(ms)) = (Kind::named(kind), ms.parse())
            {
                pending.add(kind, ms);
            }
        }
        pending
    }

    pub fn to_text(&self) -> String {
        self.times
            .iter()
            .map(|(kind, ms)| format!("{} {ms}\n", kind.name()))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// The times, the one that has waited longest first.
    pub fn times(&self) -> &[(Kind, u64)] {
        &self.times
    }

    /// The time to write for a kind of puzzle.
    pub fn of(&self, kind: Kind) -> Option<u64> {
        self.times.iter().find(|t| t.0 == kind).map(|t| t.1)
    }

    /// A time to write.  False if a time as good already waits for its
    /// kind of puzzle.
    pub fn add(&mut self, kind: Kind, ms: u64) -> bool {
        match self.times.iter_mut().find(|t| t.0 == kind) {
            Some(time) if time.1 <= ms => false,
            Some(time) => {
                time.1 = ms;
                true
            }
            None => {
                self.times.push((kind, ms));
                true
            }
        }
    }

    /// The time was written (or need not be): it waits no longer.  A
    /// better time that came meanwhile still does.  False if nothing
    /// changed.
    pub fn done(&mut self, kind: Kind, ms: u64) -> bool {
        let before = self.times.len();
        self.times.retain(|t| !(t.0 == kind && t.1 >= ms));
        self.times.len() != before
    }
}
