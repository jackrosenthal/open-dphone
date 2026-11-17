//! The best times: a table for each kind of puzzle (a size with a
//! difficulty), which the phones of a PBX share, and the times a phone
//! has yet to write there.
//!
//! A table is the value of one key of the app's store on the PBX,
//! [`Kind::key`] (e.g., `best/7x7-easy`).  It is text, a line for each
//! row, the best first: the time in milliseconds, the player's id at
//! the PBX, the player's name, and when the puzzle was solved (seconds
//! since the Unix epoch), with a tab between them.  Each solved
//! puzzle's time is a row of its own, so a player can have several, and
//! a table has at most [`ROWS`] rows.
//!
//! The times to write are text as well, a line for each time: the kind
//! of puzzle (`7x7-easy`), the milliseconds, and when the puzzle was
//! solved, with a space between them.

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

/// A solved puzzle: the time it took, and when it was solved.  When
/// tells two wins apart: written again (e.g., after a write whose answer
/// did not come), a win is the same win, and two wins of the very same
/// time are two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Win {
    /// Milliseconds.
    pub ms: u64,
    /// When the puzzle was solved: seconds since the Unix epoch, by the
    /// clock of the phone it was solved on.
    pub when: u64,
}

/// A row of a table: a player and one of their wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Milliseconds.
    pub ms: u64,
    /// When the puzzle was solved: seconds since the Unix epoch.
    pub when: u64,
    /// Identifies the player at the PBX.
    pub id: String,
    /// The player's name, to show.
    pub name: String,
}

impl Row {
    pub fn win(&self) -> Win {
        Win {
            ms: self.ms,
            when: self.when,
        }
    }
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
    /// out, and so is what a table cannot hold: a row that is another
    /// row again (the same player's same win), and rows beyond [`ROWS`].
    pub fn parse(value: &[u8]) -> Table {
        let mut table = Table::default();
        for line in String::from_utf8_lossy(value).lines() {
            let mut fields = line.split('\t');
            let (Some(ms), Some(id), Some(name), Some(when)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if let (Ok(ms), Ok(when)) = (ms.parse(), when.parse()) {
                table.add(Win { ms, when }, id, name);
            }
        }
        table
    }

    /// The table as a value of the store.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = String::new();
        for row in &self.rows {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                row.ms, row.id, row.name, row.when
            ));
        }
        text.into_bytes()
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Where a player's best row is.
    pub fn place(&self, id: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.id == id)
    }

    /// Puts a win of a player in the table, as a row of its own beside
    /// the player's other rows: after the times that are as good (who
    /// was first stays first).  False if the table is as it was: the time
    /// is not among the best [`ROWS`], or the player has a row of this
    /// very win (the same time, solved at the same moment), which is
    /// this win written before (e.g., by a write whose answer did not
    /// come).
    pub fn add(&mut self, win: Win, id: &str, name: &str) -> bool {
        let id = field(id, usize::MAX);
        let at = self.rows.iter().take_while(|r| r.ms <= win.ms).count();
        if at >= ROWS || self.rows.iter().any(|r| r.win() == win && r.id == id) {
            return false;
        }
        let name = field(name, NAME_CHARS);
        let Win { ms, when } = win;
        self.rows.insert(at, Row { ms, when, id, name });
        self.rows.truncate(ROWS);
        true
    }
}

/// The times a phone has yet to write to the tables: for each kind of
/// puzzle the best [`ROWS`] of them, since no other could make the
/// table once they are written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending {
    times: Vec<(Kind, Win)>,
}

impl Pending {
    /// What a file holds; a line that is not a time is left out.
    pub fn parse(text: &str) -> Pending {
        let mut pending = Pending::default();
        for line in text.lines() {
            let mut fields = line.split(' ');
            if let (Some(kind), Some(ms), Some(when)) =
                (fields.next(), fields.next(), fields.next())
                && let (Some(kind), Ok(ms), Ok(when)) =
                    (Kind::named(kind), ms.parse(), when.parse())
            {
                pending.add(kind, Win { ms, when });
            }
        }
        pending
    }

    pub fn to_text(&self) -> String {
        self.times
            .iter()
            .map(|(kind, win)| format!("{} {} {}\n", kind.name(), win.ms, win.when))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// The times, the one that has waited longest first.
    pub fn times(&self) -> &[(Kind, Win)] {
        &self.times
    }

    /// The times to write for a kind of puzzle, the one that has waited
    /// longest first.
    pub fn of(&self, kind: Kind) -> impl Iterator<Item = Win> + '_ {
        self.times.iter().filter(move |t| t.0 == kind).map(|t| t.1)
    }

    /// A time to write.  If [`ROWS`] times of its kind wait already, it
    /// takes the place of the worst of them (of equals, the one that has
    /// waited least), and false if each of them is as good.
    pub fn add(&mut self, kind: Kind, win: Win) -> bool {
        let of_kind = self.times.iter().enumerate().filter(|(_, t)| t.0 == kind);
        if of_kind.clone().count() >= ROWS
            && let Some((n, most)) = of_kind
                .map(|(n, t)| (n, t.1.ms))
                .max_by_key(|&(n, time)| (time, n))
        {
            if most <= win.ms {
                return false;
            }
            self.times.remove(n);
        }
        self.times.push((kind, win));
        true
    }

    /// The win was written (or need not be): it waits no longer.  The
    /// other times of its kind still do.  False if it did not wait.
    pub fn done(&mut self, kind: Kind, win: Win) -> bool {
        match self.times.iter().position(|t| *t == (kind, win)) {
            Some(n) => {
                self.times.remove(n);
                true
            }
            None => false,
        }
    }
}
