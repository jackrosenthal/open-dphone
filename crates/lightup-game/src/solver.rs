//! The solver: it counts the solutions of a grid (none, one, or more than
//! one) with three deduction rules and, where allowed, bounded guessing.
//!
//! Rule A: a dark cell that only one cell could still light gets a light
//! there.  Rule B: a clue that is full excludes the free cells beside it,
//! and a clue that needs as many lights as it has free cells beside it
//! gets them.  Rule C (set elimination): when at least one cell of a set
//! must get a light, a cell whose light would rule out every cell of the
//! set cannot hold a light.  Every conclusion holds in every solution of
//! the position, and a guess splits the solutions into those with a light
//! on a cell and those without, each searched to the end; so a count of
//! one, reached without cutting a branch off, proves the solution unique.
//!
//! A round is one pass of rules A and B over the grid; rule C gets a pass
//! only when those two have nothing more to say, and a guess is made only
//! when all three are stuck.  What the rules can conclude from a position
//! does not depend on that order (a conclusion that is open stays open,
//! or is reached another way, as lights and exclusions accumulate), so the
//! order is chosen for speed: the cheap rules first.
//!
//! A solve advances one round at a time ([`Solve::step`]), so that a
//! caller with little time per turn (the app on the phone) can spread it
//! over many turns; the result does not depend on how.

use crate::{Grid, MAX_CELLS, MAX_SIDE};

/// Which techniques a solve may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Techniques {
    /// Rule C.
    pub sets: bool,
    /// How deep guesses may nest (0: no guessing).
    pub max_depth: u8,
}

impl Techniques {
    /// Rules A and B only.
    pub const EASY: Techniques = Techniques {
        sets: false,
        max_depth: 0,
    };
    /// Rules A, B, and C.
    pub const MEDIUM: Techniques = Techniques {
        sets: true,
        max_depth: 0,
    };
    /// All rules, and guesses nested at most five deep.
    pub const HARD: Techniques = Techniques {
        sets: true,
        max_depth: 5,
    };
    /// All rules and guessing without a bound: never undetermined.
    pub const COMPLETE: Techniques = Techniques {
        sets: true,
        max_depth: u8::MAX,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The grid has no solution.
    None,
    /// Exactly one.
    Unique,
    /// Two or more.
    Several,
    /// The allowed techniques did not settle it.
    Undetermined,
}

struct Branch {
    grid: Grid,
    depth: u8,
}

/// A solve in progress.
pub struct Solve {
    tech: Techniques,
    current: Option<Branch>,
    /// The other halves of the guesses made.
    waiting: Vec<Branch>,
    found: u8,
    deepest: u8,
    solution: Option<Grid>,
    outcome: Option<Outcome>,
    rounds: u32,
}

enum Round {
    /// Something was placed or excluded.
    Progress,
    /// The position has no solution.
    Dead,
    Solved,
    /// The rules have nothing more to say, and cells are still open.
    Stuck,
}

impl Solve {
    /// A solve of the grid as it stands: lights and marks on it are taken
    /// as given.
    pub fn new(grid: &Grid, tech: Techniques) -> Solve {
        let conflict = (0..grid.cells()).any(|i| grid.light_conflicts(i));
        Solve {
            tech,
            current: (!conflict).then(|| Branch {
                grid: grid.clone(),
                depth: 0,
            }),
            waiting: Vec::new(),
            found: 0,
            deepest: 0,
            solution: None,
            outcome: None,
            rounds: 0,
        }
    }

    /// One round of deduction, or one guess.  `Some` when the solve is
    /// over.
    pub fn step(&mut self) -> Option<Outcome> {
        if self.outcome.is_some() {
            return self.outcome;
        }
        let Some(mut b) = self.current.take().or_else(|| self.waiting.pop()) else {
            self.outcome = Some(if self.found == 0 {
                Outcome::None
            } else {
                Outcome::Unique
            });
            return self.outcome;
        };
        self.rounds += 1;
        match round(&mut b.grid, self.tech.sets) {
            Round::Progress => self.current = Some(b),
            Round::Dead => {}
            Round::Solved => {
                self.found += 1;
                if self.found == 1 {
                    self.solution = Some(b.grid);
                } else {
                    self.outcome = Some(Outcome::Several);
                }
            }
            Round::Stuck if b.depth >= self.tech.max_depth => {
                // Solutions may be beyond this point: nothing can be
                // claimed about the whole.
                self.outcome = Some(Outcome::Undetermined);
            }
            Round::Stuck => {
                let cell = guess_cell(&b.grid);
                let depth = b.depth + 1;
                self.deepest = self.deepest.max(depth);
                let mut with_light = b.grid.clone();
                with_light.place_light(cell);
                self.waiting.push(Branch {
                    grid: with_light,
                    depth,
                });
                // First the half without a light there.
                b.grid.set_mark(cell, true);
                b.depth = depth;
                self.current = Some(b);
            }
        }
        self.outcome
    }

    /// Runs the solve to its end.
    pub fn run(&mut self) -> Outcome {
        loop {
            if let Some(o) = self.step() {
                return o;
            }
        }
    }

    /// The first solution found, with the marks of the deductions that
    /// led to it.
    pub fn solution(&self) -> Option<&Grid> {
        self.solution.as_ref()
    }

    /// The deepest nesting of guesses reached.
    pub fn deepest_guess(&self) -> u8 {
        self.deepest
    }

    /// How many rounds the solve has taken.
    pub fn rounds(&self) -> u32 {
        self.rounds
    }
}

/// Solves a grid as it stands.
pub fn solve(grid: &Grid, tech: Techniques) -> Outcome {
    Solve::new(grid, tech).run()
}

fn round(g: &mut Grid, sets: bool) -> Round {
    if g.is_solved() {
        return Round::Solved;
    }
    let mut changed = false;
    for i in 0..g.cells() {
        if g.clue(i).is_some() {
            if g.is_settled(i) {
                continue;
            }
            match rule_b(g, i) {
                Some(c) => changed |= c,
                None => return Round::Dead,
            }
        } else if !g.is_black(i) && !g.is_lit(i) {
            match rule_a(g, i) {
                Some(c) => changed |= c,
                None => return Round::Dead,
            }
        }
    }
    if changed {
        return Round::Progress;
    }
    if g.candidates() == 0 {
        // Not solved, and nowhere left to put a light.
        return Round::Dead;
    }
    if sets && rule_c(g) {
        return Round::Progress;
    }
    Round::Stuck
}

/// Rule A on a dark cell: `None` if nothing can light it any more.
fn rule_a(g: &mut Grid, i: usize) -> Option<bool> {
    match g.sources(i) {
        0 => None,
        1 => {
            let only = if g.is_candidate(i) {
                i
            } else {
                g.row_run(i)
                    .chain(g.column_run(i))
                    .find(|j| g.is_candidate(*j))?
            };
            g.place_light(only);
            Some(true)
        }
        _ => Some(false),
    }
}

/// The lights a clue still needs, and the cells beside it that could
/// take one.
fn clue_state(g: &Grid, i: usize) -> (i32, [u8; 4], usize) {
    let (cells, n) = g.neighbors(i);
    let mut lights = 0;
    let mut free = [0u8; 4];
    let mut nfree = 0;
    for &j in &cells[..n] {
        if g.has_light(j as usize) {
            lights += 1;
        } else if g.is_candidate(j as usize) {
            free[nfree] = j;
            nfree += 1;
        }
    }
    (g.clue(i).unwrap_or(0) as i32 - lights, free, nfree)
}

/// Rule B on a clue: `None` if the clue can no longer be met.
fn rule_b(g: &mut Grid, i: usize) -> Option<bool> {
    let (needed, free, nfree) = clue_state(g, i);
    if needed < 0 || needed > nfree as i32 {
        return None;
    }
    if nfree == 0 {
        // Met, with no cell beside it left that could take a light:
        // lights only go on such cells, so this clue stays met.
        g.set_settled(i);
        return Some(false);
    }
    if needed == 0 {
        for &j in &free[..nfree] {
            g.set_mark(j as usize, true);
        }
    } else if needed == nfree as i32 {
        for &j in &free[..nfree] {
            g.place_light(j as usize);
        }
    } else {
        return Some(false);
    }
    g.set_used(i);
    g.set_settled(i);
    Some(true)
}

/// A clue that needs exactly one more light.
fn needs_one(g: &Grid, i: usize) -> bool {
    g.clue(i)
        .is_some_and(|n| !g.is_settled(i) && g.lights_beside(i) + 1 == n as usize)
}

/// Whether a light on candidate `p` would leave candidate `q` unable to
/// take one: `p` shines on `q`, or both are beside a clue that needs
/// exactly one more light.
fn kills(g: &Grid, p: usize, q: usize) -> bool {
    if p == q {
        return false;
    }
    if g.sees(p, q) {
        return true;
    }
    // Two cells beside one clue are on opposite sides of it, or touch at
    // a corner with the clue beside both.
    let w = g.width();
    let (a, b) = (p.min(q), p.max(q));
    let (ax, bx) = (g.column(a), g.column(b));
    if b - a == 2 {
        ax + 2 == bx && needs_one(g, a + 1)
    } else if b - a == 2 * w {
        needs_one(g, a + w)
    } else if b - a == w + 1 && ax + 1 == bx {
        // `b` is below and to the right of `a`.
        needs_one(g, a + 1) || needs_one(g, a + w)
    } else if b - a == w - 1 && bx + 1 == ax {
        // Below and to the left.
        needs_one(g, a - 1) || needs_one(g, a + w)
    } else {
        false
    }
}

/// The other candidates beside the clues beside `q` that need exactly
/// one more light: a light on any of them excludes `q`.  One that shares
/// two clues with `q` is listed twice, which does no harm.
fn clue_killers(g: &Grid, q: usize, out: &mut [u8; 12]) -> usize {
    let mut n = 0;
    let (beside, nb) = g.neighbors(q);
    for &c in &beside[..nb] {
        if needs_one(g, c as usize) {
            let (others, no) = g.neighbors(c as usize);
            for &j in &others[..no] {
                if j as usize != q && g.is_candidate(j as usize) {
                    out[n] = j;
                    n += 1;
                }
            }
        }
    }
    n
}

/// Rule C on one set of candidates of which at least one must get a
/// light: excludes every candidate that kills them all.
fn eliminate(g: &mut Grid, set: &[u8]) -> bool {
    // A cell that kills the whole set is among the killers of each
    // member, so the killers of one member are enough to look through:
    // the candidates it sees, and those it shares a clue with.  The
    // member that the fewest candidates see keeps that short.
    let Some(&best) = set.iter().min_by_key(|q| g.sources(**q as usize)) else {
        return false;
    };
    let best = best as usize;
    let mut by_clue = [0u8; 12];
    let n = clue_killers(g, best, &mut by_clue);
    let mut changed = false;
    let seen = g.row_run(best).chain(g.column_run(best));
    for p in seen.chain(by_clue[..n].iter().map(|p| *p as usize)) {
        if g.is_candidate(p) && set.iter().all(|q| kills(g, p, *q as usize)) {
            g.set_mark(p, true);
            changed = true;
        }
    }
    changed
}

/// Rule C, once over all the cells: the set of every dark cell, and the
/// sets of every clue.  True if anything was excluded.
fn rule_c(g: &mut Grid) -> bool {
    // The cells beside a clue that needs exactly one more light: only
    // these can be killed through a clue.
    let mut near = [false; MAX_CELLS];
    for c in 0..g.cells() {
        if needs_one(g, c) {
            let (beside, n) = g.neighbors(c);
            for &j in &beside[..n] {
                near[j as usize] = true;
            }
        }
    }
    let mut any = false;
    for i in 0..g.cells() {
        let mut changed = false;
        if let Some(clue) = g.clue(i) {
            if clue == 0 || g.is_settled(i) {
                continue;
            }
            // With n free cells and m lights needed, at most n - m stay
            // empty: every n - m + 1 of them hold a light.
            let (m, free, n) = clue_state(g, i);
            if n == 0 || m < 1 || m > n as i32 {
                continue;
            }
            let size = n as u32 - m as u32 + 1;
            for mask in 1u32..(1 << n) {
                if mask.count_ones() != size {
                    continue;
                }
                let mut set = [0u8; 4];
                let mut len = 0;
                for (k, &cell) in free[..n].iter().enumerate() {
                    if mask & (1 << k) != 0 {
                        set[len] = cell;
                        len += 1;
                    }
                }
                changed |= eliminate(g, &set[..len]);
            }
        } else if !g.is_black(i) && !g.is_lit(i) {
            // One of the cells that could light this one must do so.
            //
            // A cell outside the two runs of this one sees one cell of
            // each run at most, and never this cell itself; whatever
            // else it kills, it kills through a clue.  So when this cell
            // could light itself and is not near such a clue, or when
            // two cells of one run are not, nothing kills the whole set,
            // and it need not be looked at any closer.
            if g.is_candidate(i) && !near[i] {
                continue;
            }
            let mut set = [0u8; 2 * MAX_SIDE];
            let mut len = 0;
            let mut hopeless = false;
            for run in 0..2 {
                let mut far = 0;
                let cells: &mut dyn Iterator<Item = usize> = if run == 0 {
                    &mut g.row_run(i)
                } else {
                    &mut g.column_run(i).filter(|j| *j != i)
                };
                for j in cells {
                    if g.is_candidate(j) {
                        set[len] = j as u8;
                        len += 1;
                        far += (!near[j] && j != i) as u32;
                    }
                }
                hopeless |= far > 1;
            }
            if hopeless {
                continue;
            }
            changed = eliminate(g, &set[..len]);
        }
        any |= changed;
    }
    any
}

/// The cell to guess on: the candidate whose light would light the most
/// dark cells; the first of them in reading order.
fn guess_cell(g: &Grid) -> usize {
    let mut best = (0, 0);
    for i in 0..g.cells() {
        if g.is_candidate(i) {
            // The cell is dark itself, and is in both of its runs.
            let gain = g
                .row_run(i)
                .chain(g.column_run(i))
                .filter(|j| !g.is_lit(*j))
                .count()
                - 1;
            if gain > best.1 {
                best = (i, gain);
            }
        }
    }
    best.0
}
