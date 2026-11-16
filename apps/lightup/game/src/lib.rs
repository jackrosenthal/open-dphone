//! Light Up, the puzzle, without the phone: the grid and its lighting, the
//! rules of play, a solver that counts solutions, a generator of puzzles
//! with exactly one solution at three difficulties, and the pictures.
//!
//! A grid has black and white cells.  The player puts lights on white
//! cells; a light shines along its row and column until a black cell or
//! the edge.  The puzzle is solved when every white cell is lit, no light
//! shines on another light, and every numbered black cell has exactly its
//! number of lights beside it.
//!
//! This is this project's own implementation, written from a prose
//! description of the method (deduction rules, bounded guessing, and
//! clue stripping); it has no dependencies and no floating point.

pub mod draw;
pub mod generate;
pub mod help;
pub mod rng;
pub mod solver;

/// The longest side of a grid.
pub const MAX_SIDE: usize = 14;
pub const MAX_CELLS: usize = MAX_SIDE * MAX_SIDE;

const BLACK: u8 = 1;
const NUMBERED: u8 = 2;
const LIGHT: u8 = 4;
/// Known (by the solver) or marked (by the player) to hold no light.
const EXCLUDED: u8 = 8;
/// The solver used this clue to place or exclude something.
const USED: u8 = 16;
/// The solver has nothing more to learn from this clue: it has its
/// lights, and no cell beside it could take another.
const SETTLED: u8 = 32;
/// What the solver notes on a cell for itself.
const NOTES: u8 = USED | SETTLED;

/// The most runs of white cells a grid can have in one direction.
const MAX_RUNS: usize = MAX_CELLS / 2;
/// The run of a black cell: one that never holds a light or a candidate.
const NO_RUN: u8 = MAX_RUNS as u8;

/// The column of each cell, for every width: the solver asks for it all
/// the time, and the phone's processor has no instruction for division.
static COLUMN: [[u8; MAX_CELLS]; MAX_SIDE + 1] = {
    let mut table = [[0u8; MAX_CELLS]; MAX_SIDE + 1];
    let mut w = 1;
    while w <= MAX_SIDE {
        let mut i = 0;
        while i < MAX_CELLS {
            table[w][i] = (i % w) as u8;
            i += 1;
        }
        w += 1;
    }
    table
};

/// The cells a cell sees along its row and column, itself not among them.
#[derive(Clone, Copy)]
pub struct Seen {
    cells: [u8; 2 * MAX_SIDE],
    len: u8,
}

impl Seen {
    pub fn as_slice(&self) -> &[u8] {
        &self.cells[..self.len as usize]
    }
}

/// A grid with its clues, lights, and no-light marks.  Cells are numbered
/// along the rows from the top left.
///
/// The white cells of a row between two black cells (or edges) are a
/// run, and likewise in a column; a light shines on exactly the two runs
/// of its cell.  The grid keeps, for every run, how many lights and how
/// many candidates (cells that could still take a light) it holds, so
/// that "is this cell lit", "does this cell see that one", and "how many
/// cells could still light this one" are answered without walking the
/// grid.
#[derive(Clone)]
pub struct Grid {
    w: u8,
    h: u8,
    flags: [u8; MAX_CELLS],
    clue: [u8; MAX_CELLS],
    /// The run of each cell along its row and along its column.
    hrun: [u8; MAX_CELLS],
    vrun: [u8; MAX_CELLS],
    /// The first cell and the length of each run.
    hstart: [u8; MAX_RUNS],
    hlen: [u8; MAX_RUNS],
    vstart: [u8; MAX_RUNS],
    vlen: [u8; MAX_RUNS],
    /// The lights in each run (one more entry, always 0, for `NO_RUN`).
    hlights: [u8; MAX_RUNS + 1],
    vlights: [u8; MAX_RUNS + 1],
    /// The candidates in each run.
    hcands: [u8; MAX_RUNS + 1],
    vcands: [u8; MAX_RUNS + 1],
    /// How many white cells no light shines on.
    dark: u16,
    /// How many candidates there are.
    cands: u16,
}

impl PartialEq for Grid {
    fn eq(&self, other: &Grid) -> bool {
        // The rest follows from these.
        let n = self.cells();
        (self.w, self.h) == (other.w, other.h)
            && self.clue == other.clue
            && self.flags[..n]
                .iter()
                .zip(&other.flags[..n])
                .all(|(a, b)| a & !NOTES == b & !NOTES)
    }
}

impl Eq for Grid {}

impl Grid {
    /// An all-white grid.
    pub fn new(width: usize, height: usize) -> Grid {
        Grid::with_black(width, height, &[false; MAX_CELLS])
    }

    /// A grid with these cells black (`black` goes along the rows).
    pub fn with_black(width: usize, height: usize, black: &[bool]) -> Grid {
        assert!((1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height));
        let mut g = Grid {
            w: width as u8,
            h: height as u8,
            flags: [0; MAX_CELLS],
            clue: [0; MAX_CELLS],
            hrun: [NO_RUN; MAX_CELLS],
            vrun: [NO_RUN; MAX_CELLS],
            hstart: [0; MAX_RUNS],
            hlen: [0; MAX_RUNS],
            vstart: [0; MAX_RUNS],
            vlen: [0; MAX_RUNS],
            hlights: [0; MAX_RUNS + 1],
            vlights: [0; MAX_RUNS + 1],
            hcands: [0; MAX_RUNS + 1],
            vcands: [0; MAX_RUNS + 1],
            dark: 0,
            cands: 0,
        };
        for (flag, black) in g.flags[..width * height].iter_mut().zip(black) {
            *flag = if *black { BLACK } else { 0 };
        }
        g.find_runs();
        g
    }

    /// Works the runs out from the black cells, and everything kept per
    /// run from the lights and marks.
    fn find_runs(&mut self) {
        let (w, n) = (self.w as usize, self.cells());
        let (mut hruns, mut vruns) = (0, 0);
        for i in 0..n {
            if self.flags[i] & BLACK != 0 {
                self.hrun[i] = NO_RUN;
                self.vrun[i] = NO_RUN;
                continue;
            }
            if self.column(i) > 0 && self.flags[i - 1] & BLACK == 0 {
                self.hrun[i] = self.hrun[i - 1];
            } else {
                self.hrun[i] = hruns as u8;
                self.hstart[hruns] = i as u8;
                self.hlen[hruns] = 0;
                hruns += 1;
            }
            self.hlen[self.hrun[i] as usize] += 1;
            if i >= w && self.flags[i - w] & BLACK == 0 {
                self.vrun[i] = self.vrun[i - w];
            } else {
                self.vrun[i] = vruns as u8;
                self.vstart[vruns] = i as u8;
                self.vlen[vruns] = 0;
                vruns += 1;
            }
            self.vlen[self.vrun[i] as usize] += 1;
        }
        self.hlights = [0; MAX_RUNS + 1];
        self.vlights = [0; MAX_RUNS + 1];
        for i in 0..n {
            if self.flags[i] & LIGHT != 0 {
                self.hlights[self.hrun[i] as usize] += 1;
                self.vlights[self.vrun[i] as usize] += 1;
            }
        }
        self.count();
    }

    /// Counts the dark cells and the candidates anew.
    fn count(&mut self) {
        self.hcands = [0; MAX_RUNS + 1];
        self.vcands = [0; MAX_RUNS + 1];
        self.dark = 0;
        self.cands = 0;
        for i in 0..self.cells() {
            if self.flags[i] & BLACK == 0 && !self.is_lit(i) {
                self.dark += 1;
                if self.flags[i] & EXCLUDED == 0 {
                    self.hcands[self.hrun[i] as usize] += 1;
                    self.vcands[self.vrun[i] as usize] += 1;
                    self.cands += 1;
                }
            }
        }
    }

    pub fn width(&self) -> usize {
        self.w as usize
    }
    pub fn height(&self) -> usize {
        self.h as usize
    }
    pub fn cells(&self) -> usize {
        self.w as usize * self.h as usize
    }
    pub fn index(&self, x: usize, y: usize) -> usize {
        y * self.w as usize + x
    }

    /// The column of a cell.
    pub fn column(&self, i: usize) -> usize {
        COLUMN[self.w as usize][i] as usize
    }

    pub fn is_black(&self, i: usize) -> bool {
        self.flags[i] & BLACK != 0
    }
    /// The number on a black cell.
    pub fn clue(&self, i: usize) -> Option<u8> {
        (self.flags[i] & NUMBERED != 0).then_some(self.clue[i])
    }
    pub fn has_light(&self, i: usize) -> bool {
        self.flags[i] & LIGHT != 0
    }
    /// The cell is marked as holding no light.
    pub fn is_marked(&self, i: usize) -> bool {
        self.flags[i] & EXCLUDED != 0
    }
    /// A white cell some light shines on (a light shines on its own cell).
    pub fn is_lit(&self, i: usize) -> bool {
        self.hlights[self.hrun[i] as usize] | self.vlights[self.vrun[i] as usize] != 0
    }
    /// How many white cells no light shines on.
    pub fn dark_cells(&self) -> usize {
        self.dark as usize
    }
    /// How many cells could still take a light.
    pub fn candidates(&self) -> usize {
        self.cands as usize
    }
    pub(crate) fn was_used(&self, i: usize) -> bool {
        self.flags[i] & USED != 0
    }
    pub(crate) fn set_used(&mut self, i: usize) {
        self.flags[i] |= USED;
    }
    pub(crate) fn is_settled(&self, i: usize) -> bool {
        self.flags[i] & SETTLED != 0
    }
    pub(crate) fn set_settled(&mut self, i: usize) {
        self.flags[i] |= SETTLED;
    }

    /// A cell where a light could still go: white, dark, and not marked.
    pub fn is_candidate(&self, i: usize) -> bool {
        self.flags[i] & (BLACK | EXCLUDED) == 0 && !self.is_lit(i)
    }

    /// Whether two white cells are in one run: a light on one shines on
    /// the other.
    pub fn sees(&self, i: usize, j: usize) -> bool {
        self.hrun[i] == self.hrun[j] || self.vrun[i] == self.vrun[j]
    }

    /// The cells of a white cell's run along its row, the cell among
    /// them.
    pub fn row_run(&self, i: usize) -> impl Iterator<Item = usize> + use<> {
        let run = self.hrun[i] as usize;
        let start = self.hstart[run] as usize;
        start..start + self.hlen[run] as usize
    }

    /// The cells of a white cell's run along its column, the cell among
    /// them.
    pub fn column_run(&self, i: usize) -> impl Iterator<Item = usize> + use<> {
        let run = self.vrun[i] as usize;
        let (start, w) = (self.vstart[run] as usize, self.w as usize);
        (0..self.vlen[run] as usize).map(move |k| start + k * w)
    }

    /// How many candidates could light a white cell: those in its two
    /// runs, the cell itself among them if it is one.
    pub fn sources(&self, i: usize) -> usize {
        self.hcands[self.hrun[i] as usize] as usize + self.vcands[self.vrun[i] as usize] as usize
            - self.is_candidate(i) as usize
    }

    /// Makes a white cell without a light black (and takes its mark).
    /// Building a grid with [`Grid::with_black`] is quicker.
    pub fn set_black(&mut self, i: usize) {
        if self.flags[i] & (BLACK | LIGHT) == 0 {
            self.flags[i] = BLACK;
            self.find_runs();
        }
    }

    /// Puts a number on a black cell, or takes it off.
    pub fn set_clue(&mut self, i: usize, clue: Option<u8>) {
        if !self.is_black(i) {
            return;
        }
        match clue {
            Some(n) => {
                self.flags[i] |= NUMBERED;
                self.clue[i] = n;
            }
            None => {
                self.flags[i] &= !(NUMBERED | NOTES);
                self.clue[i] = 0;
            }
        }
    }

    /// The cells beside a cell (up to four).
    pub fn neighbors(&self, i: usize) -> ([u8; 4], usize) {
        let w = self.w as usize;
        let x = self.column(i);
        let mut out = [0u8; 4];
        let mut n = 0;
        if i >= w {
            out[n] = (i - w) as u8;
            n += 1;
        }
        if x > 0 {
            out[n] = (i - 1) as u8;
            n += 1;
        }
        if x + 1 < w {
            out[n] = (i + 1) as u8;
            n += 1;
        }
        if i + w < self.cells() {
            out[n] = (i + w) as u8;
            n += 1;
        }
        (out, n)
    }

    /// The white cells a light on this cell would shine on, besides the
    /// cell itself.
    pub fn seen(&self, i: usize) -> Seen {
        let mut s = Seen {
            cells: [0; 2 * MAX_SIDE],
            len: 0,
        };
        if !self.is_black(i) {
            for j in self
                .row_run(i)
                .chain(self.column_run(i))
                .filter(|j| *j != i)
            {
                s.cells[s.len as usize] = j as u8;
                s.len += 1;
            }
        }
        s
    }

    /// Puts a light on a white cell (taking its mark).  False if the cell
    /// is black or has a light already.
    pub fn place_light(&mut self, i: usize) -> bool {
        if self.flags[i] & (BLACK | LIGHT) != 0 {
            return false;
        }
        let (h, v) = (self.hrun[i] as usize, self.vrun[i] as usize);
        // The cells of a run that had no light, and are in no lit run
        // across, were dark until now, and candidates unless marked.
        if self.hlights[h] == 0 {
            for j in self.row_run(i) {
                let across = self.vrun[j] as usize;
                if self.vlights[across] == 0 {
                    self.dark -= 1;
                    if self.flags[j] & EXCLUDED == 0 {
                        self.hcands[h] -= 1;
                        self.vcands[across] -= 1;
                        self.cands -= 1;
                    }
                }
            }
        }
        self.hlights[h] += 1;
        if self.vlights[v] == 0 {
            for j in self.column_run(i) {
                let across = self.hrun[j] as usize;
                if self.hlights[across] == 0 {
                    self.dark -= 1;
                    if self.flags[j] & EXCLUDED == 0 {
                        self.vcands[v] -= 1;
                        self.hcands[across] -= 1;
                        self.cands -= 1;
                    }
                }
            }
        }
        self.vlights[v] += 1;
        self.flags[i] = LIGHT;
        true
    }

    pub fn remove_light(&mut self, i: usize) -> bool {
        if !self.has_light(i) {
            return false;
        }
        self.flags[i] = 0;
        self.hlights[self.hrun[i] as usize] -= 1;
        self.vlights[self.vrun[i] as usize] -= 1;
        self.count();
        true
    }

    /// Marks a white cell without a light as holding none, or takes the
    /// mark off.  False if nothing changed.
    pub fn set_mark(&mut self, i: usize, on: bool) -> bool {
        if self.flags[i] & (BLACK | LIGHT) != 0 || self.is_marked(i) == on {
            return false;
        }
        self.flags[i] ^= EXCLUDED;
        if !self.is_lit(i) {
            // A dark cell: a candidate with the mark off, none with it on.
            let (h, v) = (self.hrun[i] as usize, self.vrun[i] as usize);
            if on {
                self.hcands[h] -= 1;
                self.vcands[v] -= 1;
                self.cands -= 1;
            } else {
                self.hcands[h] += 1;
                self.vcands[v] += 1;
                self.cands += 1;
            }
        }
        true
    }

    /// Takes every light and mark off (and what the solver noted on the
    /// clues).
    pub fn clear_play(&mut self) {
        for i in 0..self.cells() {
            if self.is_black(i) {
                self.flags[i] &= !NOTES;
            } else {
                self.flags[i] = 0;
            }
        }
        self.hlights = [0; MAX_RUNS + 1];
        self.vlights = [0; MAX_RUNS + 1];
        self.count();
    }

    /// How many lights are beside a cell.
    pub fn lights_beside(&self, i: usize) -> usize {
        let (cells, n) = self.neighbors(i);
        cells[..n]
            .iter()
            .filter(|j| self.has_light(**j as usize))
            .count()
    }

    /// A light that another light shines on.
    pub fn light_conflicts(&self, i: usize) -> bool {
        self.has_light(i)
            && self.hlights[self.hrun[i] as usize] + self.vlights[self.vrun[i] as usize] > 2
    }

    /// A numbered cell with more lights beside it than its number.
    pub fn clue_overfull(&self, i: usize) -> bool {
        self.clue(i)
            .is_some_and(|n| self.lights_beside(i) > n as usize)
    }

    /// A numbered cell that no further play can meet as things stand: it
    /// has more lights beside it than its number, or the lights beside it
    /// and the cells beside it that could still take one (white, with no
    /// light shining on them, and not marked) are fewer than its number.
    pub fn clue_unsatisfiable(&self, i: usize) -> bool {
        self.clue(i).is_some_and(|n| {
            let (cells, count) = self.neighbors(i);
            let (mut lights, mut open) = (0, 0);
            for j in &cells[..count] {
                let j = *j as usize;
                if self.has_light(j) {
                    lights += 1;
                } else if self.is_candidate(j) {
                    open += 1;
                }
            }
            lights > n as usize || lights + open < n as usize
        })
    }

    /// The three conditions of a solution; marks do not matter.
    pub fn is_solved(&self) -> bool {
        self.dark == 0
            && (0..self.cells()).all(|i| {
                if self.flags[i] & NUMBERED != 0 {
                    self.lights_beside(i) == self.clue[i] as usize
                } else {
                    !self.light_conflicts(i)
                }
            })
    }

    /// Reads a grid from rows of characters: `.` white, `#` black, `0` to
    /// `4` a numbered black cell, `L` a light, `x` a no-light mark.
    /// Spaces are ignored.  `None` if the rows are not a grid.
    pub fn parse(text: &str) -> Option<Grid> {
        let rows: Vec<Vec<char>> = text
            .lines()
            .map(|l| {
                l.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<Vec<char>>()
            })
            .filter(|l| !l.is_empty())
            .collect();
        let (h, w) = (rows.len(), rows.first()?.len());
        if h > MAX_SIDE || w > MAX_SIDE || rows.iter().any(|r| r.len() != w) {
            return None;
        }
        // Black cells first: the lights shine up to them.
        let mut black = [false; MAX_CELLS];
        for (i, c) in rows.iter().flatten().enumerate() {
            match c {
                '#' | '0'..='4' => black[i] = true,
                '.' | 'L' | 'x' => {}
                _ => return None,
            }
        }
        let mut g = Grid::with_black(w, h, &black);
        for (i, c) in rows.iter().flatten().enumerate() {
            if let '0'..='4' = c {
                g.set_clue(i, Some(*c as u8 - b'0'));
            }
        }
        for (i, c) in rows.iter().flatten().enumerate() {
            match c {
                'L' => {
                    g.place_light(i);
                }
                'x' => {
                    g.set_mark(i, true);
                }
                _ => {}
            }
        }
        Some(g)
    }

    /// The grid as rows of characters, as [`Grid::parse`] reads them.
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.cells() + self.height());
        for i in 0..self.cells() {
            out.push(match (self.is_black(i), self.clue(i)) {
                (_, Some(n)) => (b'0' + n) as char,
                (true, None) => '#',
                _ if self.has_light(i) => 'L',
                _ if self.is_marked(i) => 'x',
                _ => '.',
            });
            if (i + 1) % self.width() == 0 {
                out.push('\n');
            }
        }
        out
    }
}

impl std::fmt::Debug for Grid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\n{}", self.to_text())
    }
}

/// What the player does to the cell under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Put a light there (in place of a mark, if there is one).
    Light,
    /// Mark the cell as holding no light (in place of a light).
    Mark,
    /// Take the light or the mark off.
    Clear,
}

/// The most changes a game remembers to take back; older ones are
/// forgotten.  Written down, a change is at most five characters.
pub const UNDO_STEPS: usize = 500;

/// A puzzle being played: the grid with the player's lights and marks,
/// the cursor, and the changes made, to take back.  Once solved, it no
/// longer changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    grid: Grid,
    cursor: (u8, u8),
    solved: bool,
    /// The changes, oldest first: the cell, what it held before (`.`
    /// nothing, `L` a light, `x` a mark), and whether the change was made
    /// together with the one before it (by [`Game::auto`]), so that the
    /// two are taken back together.
    changes: Vec<(u8, u8, bool)>,
}

impl Game {
    /// A game on this grid, with the lights and marks it has, and the
    /// cursor in the top left corner.
    pub fn new(grid: Grid) -> Game {
        let solved = grid.is_solved();
        Game {
            grid,
            cursor: (0, 0),
            solved,
            changes: Vec::new(),
        }
    }

    fn held(&self, i: usize) -> u8 {
        if self.grid.has_light(i) {
            b'L'
        } else if self.grid.is_marked(i) {
            b'x'
        } else {
            b'.'
        }
    }

    /// Whether there is a change to take back.
    pub fn can_undo(&self) -> bool {
        !self.solved && !self.changes.is_empty()
    }

    /// Takes the last change back and puts the cursor on its cell; what
    /// [`Game::auto`] did at once is taken back at once, and the cursor
    /// stays where it is.  False if there is no change, or the puzzle is
    /// solved.
    pub fn undo(&mut self) -> bool {
        if self.solved || self.changes.is_empty() {
            return false;
        }
        let mut several = false;
        while let Some((cell, before, joined)) = self.changes.pop() {
            let i = cell as usize;
            self.grid.remove_light(i);
            self.grid.set_mark(i, false);
            match before {
                b'L' => {
                    self.grid.place_light(i);
                }
                b'x' => {
                    self.grid.set_mark(i, true);
                }
                _ => {}
            }
            several |= joined;
            if !joined {
                if !several {
                    self.cursor = (self.grid.column(i) as u8, (i / self.grid.width()) as u8);
                }
                break;
            }
        }
        self.solved = self.grid.is_solved();
        true
    }

    /// The changes as text, to keep with the puzzle: for each, oldest
    /// first, the number of the cell and what it held before, with a `+`
    /// in front if it was made together with the one before it.
    pub fn changes_text(&self) -> String {
        let mut out = String::new();
        for (cell, before, joined) in &self.changes {
            if *joined {
                out.push('+');
            }
            out.push_str(&cell.to_string());
            out.push(*before as char);
        }
        out
    }

    /// Takes the changes from text written by [`Game::changes_text`].
    /// Text that is anything else (a cell outside the grid, a black cell)
    /// leaves no changes to take back.
    pub fn set_changes(&mut self, text: &str) {
        self.changes.clear();
        let mut cell: Option<usize> = None;
        let mut joined = false;
        for c in text.bytes() {
            match (c, cell) {
                (b'+', None) if !joined && !self.changes.is_empty() => joined = true,
                (b'0'..=b'9', n) => {
                    let n = n.unwrap_or(0) * 10 + (c - b'0') as usize;
                    cell = Some(n.min(MAX_CELLS));
                }
                (b'.' | b'L' | b'x', Some(i))
                    if i < self.grid.cells() && !self.grid.is_black(i) =>
                {
                    self.changes.push((i as u8, c, joined));
                    cell = None;
                    joined = false;
                }
                _ => return self.changes.clear(),
            }
        }
        if cell.is_some() || joined {
            self.changes.clear();
        }
        let extra = self.changes.len().saturating_sub(UNDO_STEPS);
        self.changes.drain(..extra);
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }
    /// The cursor's column and row.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor.0 as usize, self.cursor.1 as usize)
    }
    pub fn solved(&self) -> bool {
        self.solved
    }

    /// Puts the cursor on a cell (the nearest one, if that is outside).
    pub fn set_cursor(&mut self, x: usize, y: usize) {
        self.cursor = (
            x.min(self.grid.width() - 1) as u8,
            y.min(self.grid.height() - 1) as u8,
        );
    }

    /// Moves the cursor one cell; it stops at the edges.  False if it did
    /// not move.
    pub fn move_cursor(&mut self, dx: i8, dy: i8) -> bool {
        if self.solved {
            return false;
        }
        let x = (self.cursor.0 as i8 + dx).clamp(0, self.grid.w as i8 - 1) as u8;
        let y = (self.cursor.1 as i8 + dy).clamp(0, self.grid.h as i8 - 1) as u8;
        let moved = (x, y) != self.cursor;
        self.cursor = (x, y);
        moved
    }

    /// What a key that puts a light or a mark does to the cell under the
    /// cursor: it puts that there (in place of the other, if the cell has
    /// it), or takes it off if the cell has it already.  False if nothing
    /// changed.
    pub fn toggle(&mut self, act: Act) -> bool {
        let (x, y) = self.cursor();
        let i = self.grid.index(x, y);
        let has = match act {
            Act::Light => self.grid.has_light(i),
            Act::Mark => self.grid.is_marked(i),
            Act::Clear => false,
        };
        self.act(if has { Act::Clear } else { act })
    }

    /// Acts on the cell under the cursor.  False if nothing changed: the
    /// cell is black, or already as asked, or the puzzle is solved.
    pub fn act(&mut self, act: Act) -> bool {
        let (x, y) = self.cursor();
        let i = self.grid.index(x, y);
        if self.solved || self.grid.is_black(i) {
            return false;
        }
        let before = self.held(i);
        let changed = match act {
            Act::Light => self.grid.place_light(i),
            Act::Mark => {
                let had = self.grid.remove_light(i);
                self.grid.set_mark(i, true) || had
            }
            Act::Clear => self.grid.remove_light(i) || self.grid.set_mark(i, false),
        };
        if changed {
            self.remember(i, before, false);
        }
        self.solved = self.grid.is_solved();
        changed
    }

    fn remember(&mut self, i: usize, before: u8, joined: bool) {
        if self.changes.len() == UNDO_STEPS {
            self.changes.remove(0);
        }
        self.changes.push((i as u8, before, joined));
    }

    /// Does what the numbers settle as the board stands, all at once: a
    /// no-light mark on every open cell beside a number that has its
    /// lights (or more), and a light on every open cell beside a number
    /// that needs as many more lights as it has open cells.  An open
    /// cell is one that could still take a light ([`Grid::is_candidate`]).
    ///
    /// Every number is judged on the board as it was, not on what this
    /// puts there, so lights it puts for two numbers may shine on each
    /// other; they are then shown as errors.  A cell that one number
    /// wants marked and another wants lit is left alone.  The changes are
    /// taken back together.  False if nothing changed.
    pub fn auto(&mut self) -> bool {
        if self.solved {
            return false;
        }
        const MARK: u8 = 1;
        const LIGHT: u8 = 2;
        let mut wanted = [0u8; MAX_CELLS];
        for c in 0..self.grid.cells() {
            let Some(n) = self.grid.clue(c) else { continue };
            let (cells, count) = self.grid.neighbors(c);
            let open = cells[..count]
                .iter()
                .filter(|j| self.grid.is_candidate(**j as usize));
            let lights = self.grid.lights_beside(c);
            let what = if lights >= n as usize {
                MARK
            } else if n as usize - lights == open.clone().count() {
                LIGHT
            } else {
                continue;
            };
            for j in open {
                wanted[*j as usize] |= what;
            }
        }
        let mut changed = false;
        for (i, what) in wanted.into_iter().enumerate() {
            match what {
                MARK => self.grid.set_mark(i, true),
                LIGHT => self.grid.place_light(i),
                _ => continue,
            };
            self.remember(i, b'.', changed);
            changed = true;
        }
        self.solved = self.grid.is_solved();
        changed
    }
}
