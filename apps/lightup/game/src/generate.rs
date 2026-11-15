//! The generator: puzzles with exactly one solution, at a chosen
//! difficulty.
//!
//! One attempt: scatter black cells (symmetrically), place lights at
//! random until every white cell is lit and no light sees another, and
//! number every black cell with the lights beside it.  If the solver, with
//! the techniques of the difficulty, finds that grid to have exactly one
//! solution, numbers are taken off for as long as that stays true: first
//! all the numbers the solver did not use, at once, then each remaining
//! number in turn, in a random order.  What is left is as hard as the
//! grid gets within the difficulty.  If the next easier set of techniques
//! solves it too, it is too easy and the attempt has failed.  After every
//! 20 failed attempts in a row the share of black cells goes up by 5
//! points.
//!
//! The difficulties, precisely: an easy puzzle is solved by rules A and B
//! alone; a medium one by rules A, B, and C without guessing, and not by
//! A and B alone; a hard one has one solution, proven with guesses nested
//! at most five deep, and is not solved without guessing.
//!
//! A [`Generator`] works in small steps (one round of the solver each),
//! so the caller decides how long to work at a time; the puzzle for a
//! seed does not depend on that.

use crate::rng::Rng;
use crate::solver::{Outcome, Solve, Techniques};
use crate::{Grid, MAX_CELLS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard];

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
        }
    }

    /// What the solver may use for a puzzle of this difficulty.
    pub fn techniques(self) -> Techniques {
        match self {
            Difficulty::Easy => Techniques::EASY,
            Difficulty::Medium => Techniques::MEDIUM,
            Difficulty::Hard => Techniques::HARD,
        }
    }

    fn easier(self) -> Option<Difficulty> {
        match self {
            Difficulty::Easy => None,
            Difficulty::Medium => Some(Difficulty::Easy),
            Difficulty::Hard => Some(Difficulty::Medium),
        }
    }
}

/// How the black cells repeat over the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symmetry {
    None,
    /// Top and bottom mirror each other.
    Mirror2,
    /// A half turn.
    Rotate2,
    /// Mirrored left to right and top to bottom.
    Mirror4,
    /// Quarter turns (square grids only).
    Rotate4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub width: usize,
    pub height: usize,
    /// The share of black cells, in percent, to start from.
    pub black_percent: u8,
    pub symmetry: Symmetry,
    pub difficulty: Difficulty,
}

/// The sides of the square puzzles on offer.
pub const SIZES: [usize; 3] = [7, 10, 14];

impl Params {
    /// The puzzle of a size on offer: a fifth of the cells black, laid
    /// out with quarter-turn symmetry on 7x7 and half-turn symmetry on
    /// the larger ones.
    pub fn preset(side: usize, difficulty: Difficulty) -> Params {
        Params {
            width: side,
            height: side,
            black_percent: 20,
            symmetry: if side <= 7 {
                Symmetry::Rotate4
            } else {
                Symmetry::Rotate2
            },
            difficulty,
        }
    }
}

/// Scatters black cells over an all-white grid.
pub fn black_layout(p: &Params, percent: u8, rng: &mut Rng) -> Grid {
    let (w, h) = (p.width, p.height);
    let mut black = [false; MAX_CELLS];
    let symmetry = if p.symmetry == Symmetry::Rotate4 && w != h {
        Symmetry::Rotate2
    } else {
        p.symmetry
    };
    // The part of the grid that is chosen at random; the rest follows.
    let (rw, rh) = match symmetry {
        Symmetry::None => (w, h),
        Symmetry::Mirror2 | Symmetry::Rotate2 => (w, h.div_ceil(2)),
        Symmetry::Mirror4 => (w.div_ceil(2), h.div_ceil(2)),
        Symmetry::Rotate4 => (w / 2, h.div_ceil(2)),
    };
    let wanted = rw * rh * percent as usize / 100;
    let mut placed = 0;
    while placed < wanted {
        let x = rng.below(rw as u32) as usize;
        let y = rng.below(rh as u32) as usize;
        if black[y * w + x] {
            continue;
        }
        placed += 1;
        let (mx, my) = (w - 1 - x, h - 1 - y);
        let images: [(usize, usize); 4] = match symmetry {
            Symmetry::None => [(x, y); 4],
            Symmetry::Mirror2 => [(x, y), (x, my), (x, y), (x, my)],
            Symmetry::Rotate2 => [(x, y), (mx, my), (x, y), (mx, my)],
            Symmetry::Mirror4 => [(x, y), (mx, y), (x, my), (mx, my)],
            Symmetry::Rotate4 => [(x, y), (my, x), (mx, my), (y, mx)],
        };
        for (ix, iy) in images {
            black[iy * w + ix] = true;
        }
    }
    // Quarter turns of an odd square leave the center to itself.
    if symmetry == Symmetry::Rotate4 && w % 2 == 1 && rng.below(100) < percent as u32 {
        black[(h / 2) * w + w / 2] = true;
    }
    Grid::with_black(w, h, &black)
}

/// Places lights on a grid without any until every white cell is lit and
/// no light shines on another: the white cells in a random order, each
/// getting a light if no light shines on it yet.
pub fn place_lights(g: &mut Grid, rng: &mut Rng) {
    let mut order = [0u8; MAX_CELLS];
    let n = g.cells();
    for (i, o) in order[..n].iter_mut().enumerate() {
        *o = i as u8;
    }
    rng.shuffle(&mut order[..n]);
    for &i in &order[..n] {
        if !g.is_black(i as usize) && !g.is_lit(i as usize) {
            g.place_light(i as usize);
        }
    }
}

/// Numbers every black cell with the lights beside it.
pub fn number_all(g: &mut Grid) {
    for i in 0..g.cells() {
        if g.is_black(i) {
            g.set_clue(i, Some(g.lights_beside(i) as u8));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Working,
    /// The puzzle is ready ([`Generator::puzzle`]).
    Done,
    /// The allowed number of attempts is used up.
    Failed,
}

enum Stage {
    /// A new grid is due.
    Fresh,
    /// Solving the grid with every black cell numbered.
    Full,
    /// Solving it without the numbers the first solve did not use.
    Bulk,
    /// Solving it without the number at this place of the order.
    Strip(usize),
    /// Solving the finished puzzle with the next easier techniques.
    TooEasy,
    Done,
    Failed,
}

pub struct Generator {
    params: Params,
    rng: Rng,
    /// The order in which numbers are taken off, the same for every
    /// attempt.
    order: Vec<u8>,
    percent: u8,
    attempts: u32,
    max_attempts: u32,
    stage: Stage,
    /// The puzzle so far: black cells and numbers only.
    puzzle: Grid,
    /// The puzzle with something taken off, being tested.
    trial: Grid,
    solve: Option<Solve>,
    steps: u64,
}

impl Generator {
    /// A generator that gives up after `max_attempts` grids.
    pub fn new(params: Params, seed: u64, max_attempts: u32) -> Generator {
        let mut rng = Rng::new(seed);
        let mut order: Vec<u8> = (0..(params.width * params.height) as u8).collect();
        rng.shuffle(&mut order);
        let empty = Grid::new(params.width, params.height);
        Generator {
            params,
            rng,
            order,
            percent: params.black_percent,
            attempts: 0,
            max_attempts,
            stage: Stage::Fresh,
            puzzle: empty.clone(),
            trial: empty,
            solve: None,
            steps: 0,
        }
    }

    /// Grids tried so far, counting the one in the works.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Steps taken so far.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The finished puzzle: black cells and numbers, no lights.
    pub fn puzzle(&self) -> Option<&Grid> {
        matches!(self.stage, Stage::Done).then_some(&self.puzzle)
    }

    fn test(&mut self, difficulty: Difficulty) {
        self.solve = Some(Solve::new(&self.trial, difficulty.techniques()));
    }

    /// The attempt failed: the next grid, with more black cells after
    /// every 20 failures.
    fn fail(&mut self) {
        if self.attempts.is_multiple_of(20) && self.percent < 90 {
            self.percent += 5;
        }
        self.stage = if self.attempts >= self.max_attempts {
            Stage::Failed
        } else {
            Stage::Fresh
        };
    }

    /// Takes the next number off, from place `from` of the order on, and
    /// starts the test without it; with none left, the puzzle is as hard
    /// as it gets.
    fn strip_from(&mut self, from: usize) {
        for k in from..self.order.len() {
            let i = self.order[k] as usize;
            if self.puzzle.clue(i).is_some() {
                self.trial = self.puzzle.clone();
                self.trial.set_clue(i, None);
                self.test(self.params.difficulty);
                self.stage = Stage::Strip(k);
                return;
            }
        }
        match self.params.difficulty.easier() {
            Some(easier) => {
                self.trial = self.puzzle.clone();
                self.test(easier);
                self.stage = Stage::TooEasy;
            }
            None => self.stage = Stage::Done,
        }
    }

    /// A small piece of the work: a new grid, or one round of a solve.
    pub fn step(&mut self) -> Progress {
        self.steps += 1;
        if let Stage::Fresh = self.stage {
            self.attempts += 1;
            let mut g = black_layout(&self.params, self.percent, &mut self.rng);
            place_lights(&mut g, &mut self.rng);
            debug_assert!(g.dark_cells() == 0 && (0..g.cells()).all(|i| !g.light_conflicts(i)));
            number_all(&mut g);
            g.clear_play();
            self.puzzle = g;
            self.trial = self.puzzle.clone();
            self.test(self.params.difficulty);
            self.stage = Stage::Full;
            return Progress::Working;
        }
        let outcome = match (&self.stage, &mut self.solve) {
            (Stage::Done, _) => return Progress::Done,
            (Stage::Failed, _) | (_, None) => return Progress::Failed,
            (_, Some(solve)) => match solve.step() {
                Some(outcome) => outcome,
                None => return Progress::Working,
            },
        };
        let unique = outcome == Outcome::Unique;
        match self.stage {
            Stage::Full if unique => {
                // Without the numbers this solve never used: often most
                // of the spare ones, for the price of one more solve.
                let solved = self
                    .solve
                    .as_ref()
                    .and_then(Solve::solution)
                    .expect("a solution");
                self.trial = self.puzzle.clone();
                for i in 0..self.trial.cells() {
                    if !solved.was_used(i) {
                        self.trial.set_clue(i, None);
                    }
                }
                self.test(self.params.difficulty);
                self.stage = Stage::Bulk;
            }
            Stage::Full => self.fail(),
            Stage::Bulk => {
                if unique {
                    self.puzzle = self.trial.clone();
                }
                self.strip_from(0);
            }
            Stage::Strip(k) => {
                if unique {
                    self.puzzle = self.trial.clone();
                }
                self.strip_from(k + 1);
            }
            Stage::TooEasy if unique => self.fail(),
            Stage::TooEasy => self.stage = Stage::Done,
            Stage::Fresh | Stage::Done | Stage::Failed => {}
        }
        match self.stage {
            Stage::Done => Progress::Done,
            Stage::Failed => Progress::Failed,
            _ => Progress::Working,
        }
    }
}

/// Generates a puzzle in one go.  `None` if `max_attempts` grids did not
/// yield one.
pub fn generate(params: Params, seed: u64, max_attempts: u32) -> Option<Grid> {
    let mut g = Generator::new(params, seed, max_attempts);
    loop {
        match g.step() {
            Progress::Working => {}
            Progress::Done => return g.puzzle().cloned(),
            Progress::Failed => return None,
        }
    }
}
