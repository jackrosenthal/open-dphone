//! Box Pusher: a figure on a grid pushes boxes, one at a time, onto the
//! marked squares.  A box cannot be pulled, and neither a wall nor a second
//! box gives way.
//!
//! This crate is the game without the phone: the rules ([`Game`]), the
//! levels ([`LEVELS`]), and the pictures ([`draw`]).  The app in
//! `apps/boxpusher` puts them on the screen.  The levels are this
//! project's own; `tests/levels.rs` holds the tool that made them and the
//! solver that proves each one solvable in the recorded number of pushes.

pub mod draw;
mod levels;

pub use levels::LEVELS;

/// The largest level, in squares, walls included.  A level this size fits
/// the smallest display (see [`draw::Layout`]).
pub const MAX_WIDTH: usize = 11;
pub const MAX_HEIGHT: usize = 7;

/// A level as it is written down, and the fewest pushes that solve it.
#[derive(Clone, Copy, Debug)]
pub struct LevelDef {
    /// One line of text for each row: `#` wall, `.` floor, `o` box,
    /// `x` target, `O` box on a target, `p` the figure, `P` the figure on
    /// a target, and a space for a square outside the walls.
    pub text: &'static str,
    /// The fewest pushes that solve the level.
    pub pushes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    /// The step in columns and rows.
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    /// The letter a recorded move is written with.
    pub fn letter(self) -> char {
        match self {
            Dir::Up => 'u',
            Dir::Down => 'd',
            Dir::Left => 'l',
            Dir::Right => 'r',
        }
    }

    pub fn from_letter(c: char) -> Option<Dir> {
        Dir::ALL.into_iter().find(|d| d.letter() == c)
    }
}

/// A level: the walls, the targets, and where everything starts.  Squares
/// are numbered row by row from the top left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub walls: Vec<bool>,
    pub targets: Vec<bool>,
    pub boxes: Vec<bool>,
    pub player: usize,
}

impl Level {
    /// Reads a level (see [`LevelDef::text`]) and checks that it can be
    /// played: no larger than the limit, one figure, as many boxes as
    /// targets, and walls all the way around everything the figure can
    /// reach.
    pub fn parse(text: &str) -> Result<Level, String> {
        let rows: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let height = rows.len();
        let width = rows.iter().map(|r| r.trim_end().len()).max().unwrap_or(0);
        if width == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            return Err(format!("{width}x{height} squares is not a level size"));
        }
        let n = width * height;
        let mut level = Level {
            width,
            height,
            // What is not written is outside, which nothing can enter.
            walls: vec![true; n],
            targets: vec![false; n],
            boxes: vec![false; n],
            player: n,
        };
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.trim_end().chars().enumerate() {
                let i = y * width + x;
                match c {
                    '#' | ' ' => continue,
                    '.' => {}
                    'o' => level.boxes[i] = true,
                    'x' => level.targets[i] = true,
                    'O' => (level.boxes[i], level.targets[i]) = (true, true),
                    'p' | 'P' => {
                        if level.player != n {
                            return Err("more than one figure".into());
                        }
                        level.player = i;
                        level.targets[i] = c == 'P';
                    }
                    other => return Err(format!("unknown square {other:?}")),
                }
                level.walls[i] = false;
                if x == 0 || y == 0 || x == width - 1 || y == height - 1 {
                    return Err("the floor reaches the edge".into());
                }
            }
        }
        if level.player == n {
            return Err("no figure".into());
        }
        let boxes = level.boxes.iter().filter(|b| **b).count();
        let targets = level.targets.iter().filter(|t| **t).count();
        if boxes == 0 || boxes != targets {
            return Err(format!("{boxes} boxes for {targets} targets"));
        }
        let inside = level.inside();
        if (0..n).any(|i| (level.boxes[i] || level.targets[i]) && !inside[i]) {
            return Err("a box or a target is out of reach".into());
        }
        Ok(level)
    }

    /// The square one step from `i`.  Floor never touches the edge, so a
    /// step from a floor square stays on the grid.
    pub fn step(&self, i: usize, dir: Dir) -> usize {
        let (dx, dy) = dir.delta();
        (i as i32 + dx + dy * self.width as i32) as usize
    }

    /// The squares the figure could reach if there were no boxes: the
    /// floor of the level, as opposed to floor shut off behind walls.
    pub fn inside(&self) -> Vec<bool> {
        let mut seen = vec![false; self.walls.len()];
        let mut stack = vec![self.player];
        seen[self.player] = true;
        while let Some(i) = stack.pop() {
            for d in Dir::ALL {
                let j = self.step(i, d);
                if !self.walls[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        seen
    }

    pub fn box_count(&self) -> usize {
        self.boxes.iter().filter(|b| **b).count()
    }
}

/// What a key press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A wall, or a box that cannot move, is in the way.
    Blocked,
    Moved,
    Pushed,
}

/// A level being played.
#[derive(Clone, Debug)]
pub struct Game {
    level: Level,
    boxes: Vec<bool>,
    player: usize,
    /// Every step taken, and whether it pushed a box.
    history: Vec<(Dir, bool)>,
    pushes: u32,
}

impl Game {
    pub fn new(level: Level) -> Game {
        Game {
            boxes: level.boxes.clone(),
            player: level.player,
            level,
            history: Vec::new(),
            pushes: 0,
        }
    }

    pub fn level(&self) -> &Level {
        &self.level
    }
    pub fn player(&self) -> usize {
        self.player
    }
    pub fn has_box(&self, i: usize) -> bool {
        self.boxes[i]
    }
    pub fn moves(&self) -> u32 {
        self.history.len() as u32
    }
    pub fn pushes(&self) -> u32 {
        self.pushes
    }

    /// Every box is on a target.
    pub fn solved(&self) -> bool {
        self.boxes
            .iter()
            .zip(&self.level.targets)
            .all(|(b, t)| !b || *t)
    }

    /// One step of the figure, pushing the box in front of it if there is
    /// one and the square behind that box is free.
    pub fn step(&mut self, dir: Dir) -> Step {
        let to = self.level.step(self.player, dir);
        if self.level.walls[to] {
            return Step::Blocked;
        }
        let pushed = self.boxes[to];
        if pushed {
            let beyond = self.level.step(to, dir);
            if self.level.walls[beyond] || self.boxes[beyond] {
                return Step::Blocked;
            }
            self.boxes[to] = false;
            self.boxes[beyond] = true;
            self.pushes += 1;
        }
        self.player = to;
        self.history.push((dir, pushed));
        if pushed { Step::Pushed } else { Step::Moved }
    }

    /// Takes the last step back, with the box it pushed.  False if there
    /// is nothing to take back.
    pub fn undo(&mut self) -> bool {
        let Some((dir, pushed)) = self.history.pop() else {
            return false;
        };
        let (dx, dy) = dir.delta();
        let back = (self.player as i32 - dx - dy * self.level.width as i32) as usize;
        if pushed {
            let at = self.level.step(self.player, dir);
            self.boxes[at] = false;
            self.boxes[self.player] = true;
            self.pushes -= 1;
        }
        self.player = back;
        true
    }

    /// Back to the start of the level.
    pub fn restart(&mut self) {
        *self = Game::new(self.level.clone());
    }

    /// The steps taken so far, one letter each (see [`Dir::letter`]).
    pub fn record(&self) -> String {
        self.history.iter().map(|(d, _)| d.letter()).collect()
    }

    /// Takes the steps of a record, stopping at the first one that is not
    /// a letter or is blocked.  Returns how many were taken.
    pub fn replay(&mut self, record: &str) -> usize {
        let mut taken = 0;
        for c in record.chars() {
            match Dir::from_letter(c) {
                Some(d) if self.step(d) != Step::Blocked => taken += 1,
                _ => break,
            }
        }
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One box and two targets would not parse, so the room has a second
    /// box already home in the corner.
    const ROOM: &str = "\
#######
#p.o.x#
#..#..#
#O....#
#######";

    fn game() -> Game {
        Game::new(Level::parse(ROOM).unwrap())
    }

    fn at(g: &Game, x: usize, y: usize) -> usize {
        y * g.level().width + x
    }

    #[test]
    fn parses_what_is_written() {
        let l = Level::parse(ROOM).unwrap();
        assert_eq!((l.width, l.height, l.player), (7, 5, 8));
        assert_eq!(l.box_count(), 2);
        assert!(l.boxes[10] && l.targets[12] && l.boxes[22] && l.targets[22]);
        assert!(l.walls[0] && l.walls[17] && !l.walls[9]);
    }

    #[test]
    fn refuses_what_cannot_be_played() {
        let bad = [
            ("", "size"),
            ("############\n#p.o.x.....#\n############", "size"),
            ("###\n#p#\n#o#\n#x#\n#.#\n#.#\n#.#\n###", "size"),
            ("####\n#ox#\n####", "no figure"),
            ("#####\n#pPo#\n#####", "more than one"),
            ("#####\n#p.o#\n#####", "1 boxes for 0 targets"),
            ("#####\n#p.x#\n#####", "0 boxes for 1 targets"),
            ("####\n#pox\n####", "edge"),
            ("#######\n#po#x##\n#######", "out of reach"),
            ("####\n#p?#\n####", "unknown"),
        ];
        for (text, why) in bad {
            let e = Level::parse(text).unwrap_err();
            assert!(e.contains(why), "{text:?}: {e}");
        }
    }

    #[test]
    fn walks_and_stops_at_walls() {
        let mut g = game();
        assert_eq!(g.step(Dir::Up), Step::Blocked);
        assert_eq!(g.step(Dir::Left), Step::Blocked);
        assert_eq!((g.moves(), g.pushes(), g.player()), (0, 0, at(&g, 1, 1)));
        assert_eq!(g.step(Dir::Down), Step::Moved);
        assert_eq!(g.step(Dir::Right), Step::Moved);
        // The pillar in the middle of the room.
        assert_eq!(g.step(Dir::Right), Step::Blocked);
        assert_eq!((g.moves(), g.pushes(), g.player()), (2, 0, at(&g, 2, 2)));
    }

    #[test]
    fn pushes_one_box_and_never_two_or_into_a_wall() {
        let mut g = game();
        assert_eq!(g.step(Dir::Right), Step::Moved);
        assert_eq!(g.step(Dir::Right), Step::Pushed);
        assert!(g.has_box(at(&g, 4, 1)) && !g.has_box(at(&g, 3, 1)));
        assert_eq!((g.moves(), g.pushes()), (2, 1));
        assert!(!g.solved());
        assert_eq!(g.step(Dir::Right), Step::Pushed);
        assert!(g.solved(), "both boxes are on targets");
        // The box is against the wall now.
        assert_eq!(g.step(Dir::Right), Step::Blocked);
        assert_eq!((g.moves(), g.pushes(), g.player()), (3, 2, at(&g, 4, 1)));

        // The box in the corner is against the wall from the start.
        let mut g = game();
        g.replay("d");
        assert_eq!(g.step(Dir::Down), Step::Blocked);
        assert_eq!((g.moves(), g.pushes()), (1, 0));

        let mut g = Game::new(Level::parse("######\n#poox#\n#x...#\n######").unwrap());
        assert_eq!(g.step(Dir::Right), Step::Blocked, "two boxes in a row");
        assert_eq!((g.moves(), g.pushes()), (0, 0));
    }

    #[test]
    fn undo_takes_back_steps_and_pushes() {
        let mut g = game();
        assert!(!g.undo());
        g.replay("rrr");
        assert!(g.solved());
        assert!(g.undo());
        assert!(!g.solved());
        assert_eq!((g.moves(), g.pushes(), g.player()), (2, 1, at(&g, 3, 1)));
        assert!(g.has_box(at(&g, 4, 1)) && !g.has_box(at(&g, 5, 1)));
        assert!(g.undo());
        assert!(g.undo());
        assert!(!g.undo());
        assert_eq!((g.moves(), g.pushes(), g.player()), (0, 0, at(&g, 1, 1)));
        assert_eq!(g.boxes, g.level().boxes);
        // A step that did not push leaves the box where it is.
        g.replay("dr");
        assert!(g.undo());
        assert_eq!(g.player(), at(&g, 1, 2));
        assert_eq!(g.boxes, g.level().boxes);
    }

    #[test]
    fn restart_and_record() {
        let mut g = game();
        g.replay("rrr");
        assert_eq!(g.record(), "rrr");
        g.restart();
        assert_eq!((g.moves(), g.pushes(), g.player()), (0, 0, at(&g, 1, 1)));
        assert!(!g.solved());
        assert_eq!(g.record(), "");
        assert_eq!(g.replay("rr?r"), 2);
        assert_eq!(g.record(), "rr");
    }
}
