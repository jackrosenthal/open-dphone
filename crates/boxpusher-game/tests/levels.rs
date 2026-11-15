//! The levels, proven: a solver finds the fewest pushes for every level
//! and it must be the number the level records, and the solution written
//! down here for each level must solve it when played through the rules.
//!
//! The same file holds the tool the levels were made with (`make_levels`,
//! not run by default).  It builds random rooms, puts the boxes on the
//! targets, and plays backward (pulling boxes) to every position that can
//! be reached; the position that is hardest to solve is the level, and
//! the tool prints the hardest levels of all the rooms it tried.  Levels
//! 5 to 28 were picked by hand from what it printed for rooms of
//! different sizes with one to four boxes, and put in the order of their
//! pushes; levels 1 to 4 were laid out by hand.  Nothing is taken from
//! any other game or level collection.
//!
//! ```text
//! BOXES=3 WIDTH=9 HEIGHT=7 DENSITY=7 ROOMS=8000 SEED=5 \
//!   cargo test --release -p boxpusher-game --test levels -- --ignored --nocapture make_levels
//! cargo test --release -p boxpusher-game --test levels -- --ignored --nocapture print_solutions
//! ```

use boxpusher_game::{Dir, Game, LEVELS, Level, MAX_HEIGHT, MAX_WIDTH};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};

/// A solution of each level in [`LEVELS`] with the fewest pushes, as the
/// solver found it (`print_solutions`).
const SOLUTIONS: &[&str] = &[
    // 1: 2 pushes
    "rrr",
    // 2: 2 pushes
    "ururd",
    // 3: 3 pushes
    "rdldrr",
    // 4: 4 pushes
    "drrrdru",
    // 5: 6 pushes
    "lldrrurrddlurulll",
    // 6: 7 pushes
    "ddrrrurrdllll",
    // 7: 8 pushes
    "rrrurrdlllll",
    // 8: 9 pushes
    "ulluldrrrddlurululldrrlllddrulurr",
    // 9: 10 pushes
    "ulldrrurdlllddruuruldrrrddlurulll",
    // 10: 12 pushes
    "uuuldllurrdrddlurululldrrdrruullrdd",
    // 11: 13 pushes
    "dddrurrdrruldlullluurdldrrrdrrullll",
    // 12: 13 pushes
    "rrrrrdrruuldldlldlllururrrrrrdlllluldrdl",
    // 13: 14 pushes
    "uuruuldllulddurrrrddlurulllulldrrrrlldlldr",
    // 14: 16 pushes
    "drurrdrulllluurdldrrrdrrulllluurrrdullldddlurrrdrruluurd",
    // 15: 17 pushes
    "luurrrurrdluldlllddrulurrrurrdllllulddurrrrrrddllludrrruullll",
    // 16: 17 pushes
    "drrrdrruuulldurrdllldlluurrdrrrddllulurdlllurdrrururdlldlluurrrllldrrr",
    // 17: 18 pushes
    "uuuullldllurrrrdrrulldlluldrrrulldldurrrrddluruulldrlllddldr",
    // 18: 19 pushes
    "uurrrlllddrrrurruuldllluldrrrrrdlrdllllrrruuldrdll",
    // 19: 19 pushes
    "rrrrrrrdllullddrurullluldrrdddrruurulllllulddduurrrdddrrrurul",
    // 20: 21 pushes
    "rrruululldrrrrrrdrrullllldddruulurrrdrrullll",
    // 21: 22 pushes
    "dlddllluuluurdldldrrrrllluurdldrrdrruulurrrdllddllurdruulurddllllldr",
    // 22: 23 pushes
    "lluurdurdurrdllluldrrrrddlludrruuullldrrurdlllllddrulurrrdlullddrulururrrrdduulldrullldrrurrdulldr",
    // 23: 24 pushes
    "ullllrrdrdlllulluurdrrrdrruuldllllldrurrrddllulurrrrlddrruluurdlllldlldrrrr",
    // 24: 26 pushes
    "ullldddlluuluurdldddrrruuullrrrrdrddluurullldddllululuurrdrullldrurrdrlulldrldddrrruuurrurdllldddlluluurrrrrlldddlluulurrrr",
    // 25: 27 pushes
    "llldlllrrdrdlllulldrrrruuulldrrddlllulurrrurrddluruldddllulurrrdrulurrrdddlllll",
    // 26: 28 pushes
    "rrruuurdurrdllulddurrdlrrddllulllulluurrrrdduullllddddruruldluluurrrrrdrrdrddllulllullddrurulldludrrrrrrdrrulllllluld",
    // 27: 28 pushes
    "lrdddldlluudrrrrdllulluluurdrrurdddlllulurrrurdlllurrdrdulllddrrrrrllldrrr",
    // 28: 33 pushes
    "dddlllluurduluurdlddrrdrullluurdlddlllrrrrrulrrruuldrdlldlluuurulllddrrddrruldluurullldrrdrrrrdlll",
];

const NONE: u8 = 255;

/// The floor of a level, numbered so that a set of squares is the bits of
/// a `u64`.
struct Map {
    /// The level's square of each floor square.
    cell: Vec<usize>,
    /// The floor square of each of the level's squares.
    index: Vec<u8>,
    /// The neighbors in the order of `Dir::ALL` (so `d ^ 1` is the
    /// opposite of `d`).
    adj: Vec<[u8; 4]>,
    targets: u64,
    /// Squares from which a box can never reach a target, even alone.
    dead: u64,
}

impl Map {
    fn new(level: &Level) -> Map {
        let inside = level.inside();
        let cell: Vec<usize> = (0..inside.len()).filter(|i| inside[*i]).collect();
        assert!(cell.len() <= 64, "too much floor for the solver");
        let mut index = vec![NONE; inside.len()];
        for (n, c) in cell.iter().enumerate() {
            index[*c] = n as u8;
        }
        let adj: Vec<[u8; 4]> = cell
            .iter()
            .map(|c| Dir::ALL.map(|d| index[level.step(*c, d)]))
            .collect();
        let targets = bits(cell.iter().map(|c| level.targets[*c]));
        // A box alone can be on a square if it can be pulled there from
        // a target.
        let mut alive = targets;
        let mut todo: Vec<u8> = (0..cell.len() as u8).filter(|i| targets >> i & 1 == 1).collect();
        while let Some(at) = todo.pop() {
            for (d, from) in adj[at as usize].into_iter().enumerate() {
                if from == NONE || alive >> from & 1 == 1 {
                    continue;
                }
                // Pushed from `from` to `at`, the figure stood behind it.
                if adj[from as usize][d] != NONE {
                    alive |= 1 << from;
                    todo.push(from);
                }
            }
        }
        let dead = !alive & (u64::MAX >> (64 - cell.len()));
        Map { cell, index, adj, targets, dead }
    }

    /// The squares the figure can walk to from `from` between `boxes`.
    fn reach(&self, boxes: u64, from: u8) -> u64 {
        let mut seen = 1u64 << from;
        let mut stack = vec![from];
        while let Some(i) = stack.pop() {
            for j in self.adj[i as usize] {
                if j != NONE && (seen | boxes) >> j & 1 == 0 {
                    seen |= 1 << j;
                    stack.push(j);
                }
            }
        }
        seen
    }

    /// The steps of a shortest walk between two squares of one region.
    fn walk(&self, boxes: u64, from: u8, to: u8) -> String {
        let mut came: HashMap<u8, (u8, usize)> = HashMap::new();
        let mut queue = VecDeque::from([from]);
        while let Some(i) = queue.pop_front() {
            if i == to {
                break;
            }
            for (d, j) in self.adj[i as usize].into_iter().enumerate() {
                if j != NONE && j != from && boxes >> j & 1 == 0 && !came.contains_key(&j) {
                    came.insert(j, (i, d));
                    queue.push_back(j);
                }
            }
        }
        let mut steps = Vec::new();
        let mut at = to;
        while at != from {
            let (prev, d) = came[&at];
            steps.push(Dir::ALL[d].letter());
            at = prev;
        }
        steps.iter().rev().collect()
    }
}

fn bits(flags: impl Iterator<Item = bool>) -> u64 {
    flags.enumerate().fold(0, |m, (i, f)| m | (f as u64) << i)
}

/// A position: where the boxes are, and the lowest-numbered square of the
/// region the figure is in (which square of its region it stands on makes
/// no difference to what can be pushed).
type State = (u64, u8);

/// The fewest pushes that solve the level, and a solution with that many,
/// or `None` if there is no solution.  Breadth-first over pushes.
fn solve(level: &Level) -> Option<(u32, String)> {
    let map = Map::new(level);
    let boxes = bits(map.cell.iter().map(|c| level.boxes[*c]));
    let player = map.index[level.player];
    let start: State = (boxes, map.reach(boxes, player).trailing_zeros() as u8);
    // For each position: the position before, the box that was pushed,
    // and which way.
    let mut from: HashMap<State, (State, u8, u8)> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    let mut end = None;
    while let Some(state) = queue.pop_front() {
        let (boxes, at) = state;
        if boxes == map.targets {
            end = Some(state);
            break;
        }
        let reach = map.reach(boxes, at);
        let mut left = boxes;
        while left != 0 {
            let b = left.trailing_zeros() as u8;
            left &= left - 1;
            for d in 0..4 {
                let (to, stand) = (map.adj[b as usize][d], map.adj[b as usize][d ^ 1]);
                if to == NONE || stand == NONE || reach >> stand & 1 == 0 {
                    continue;
                }
                if (boxes | map.dead) >> to & 1 == 1 {
                    continue;
                }
                let moved = boxes & !(1 << b) | 1 << to;
                let next: State = (moved, map.reach(moved, b).trailing_zeros() as u8);
                if next != start && !from.contains_key(&next) {
                    from.insert(next, (state, b, d as u8));
                    queue.push_back(next);
                }
            }
        }
    }
    let mut pushes = Vec::new();
    let mut state = end?;
    while state != start {
        let (prev, b, d) = from[&state];
        pushes.push((prev.0, b, d));
        state = prev;
    }
    pushes.reverse();
    let mut steps = String::new();
    let mut at = player;
    for (boxes, b, d) in &pushes {
        let stand = map.adj[*b as usize][(*d ^ 1) as usize];
        steps += &map.walk(*boxes, at, stand);
        steps.push(Dir::ALL[*d as usize].letter());
        at = *b;
    }
    Some((pushes.len() as u32, steps))
}

/// Whether a room uses its whole width and height: there is floor in the
/// first and the last row and column inside the outer wall.
fn fills(wall: &[bool], w: usize) -> bool {
    let h = wall.len() / w;
    let floor = |mut cells: Box<dyn Iterator<Item = usize>>| cells.any(|c| !wall[c]);
    floor(Box::new((0..h).map(move |y| y * w + 1)))
        && floor(Box::new((0..h).map(move |y| y * w + w - 2)))
        && floor(Box::new(w..2 * w))
        && floor(Box::new((h - 2) * w..(h - 1) * w))
}

fn levels() -> Vec<Level> {
    LEVELS
        .iter()
        .enumerate()
        .map(|(i, def)| Level::parse(def.text).unwrap_or_else(|e| panic!("level {}: {e}", i + 1)))
        .collect()
}

#[test]
fn there_are_twenty_to_thirty_levels_that_fit_the_smallest_display() {
    assert!((20..=30).contains(&LEVELS.len()), "{} levels", LEVELS.len());
    for (i, level) in levels().iter().enumerate() {
        assert!(level.width <= MAX_WIDTH && level.height <= MAX_HEIGHT, "level {}", i + 1);
        // No row or column of nothing around the level.
        let inside = level.inside();
        let walls: Vec<bool> = inside.iter().map(|f| !f).collect();
        assert!(fills(&walls, level.width), "level {}", i + 1);
    }
}

#[test]
fn every_level_is_solved_by_its_recorded_solution() {
    assert_eq!(SOLUTIONS.len(), LEVELS.len());
    for (i, level) in levels().into_iter().enumerate() {
        let mut game = Game::new(level);
        assert!(!game.solved(), "level {} starts solved", i + 1);
        let steps = SOLUTIONS[i];
        assert_eq!(game.replay(steps), steps.len(), "level {}: a step is blocked", i + 1);
        assert!(game.solved(), "level {}: not solved", i + 1);
        assert_eq!(game.pushes(), LEVELS[i].pushes, "level {}", i + 1);
        // The solution does not go on after the level is solved.
        assert!(game.undo());
        assert!(!game.solved(), "level {}", i + 1);
    }
}

#[test]
fn the_solver_finds_the_recorded_fewest_pushes() {
    for (i, level) in levels().iter().enumerate() {
        let (pushes, steps) = solve(level).unwrap_or_else(|| panic!("level {} has no solution", i + 1));
        assert_eq!(pushes, LEVELS[i].pushes, "level {}", i + 1);
        // What the solver found is itself a solution under the rules.
        let mut game = Game::new(level.clone());
        assert_eq!(game.replay(&steps), steps.len(), "level {}", i + 1);
        assert!(game.solved() && game.pushes() == pushes, "level {}", i + 1);
    }
}

#[test]
fn the_solver_knows_an_impossible_level() {
    // The box is in a corner.
    assert_eq!(solve(&Level::parse("#####\n#o.x#\n#..p#\n#####").unwrap()), None);
    // Two boxes: one push for the first and two for the second.
    let level = Level::parse("######\n#p.ox#\n#.o.x#\n######").unwrap();
    assert_eq!(solve(&level).map(|s| s.0), Some(3));
}

#[test]
fn the_levels_start_trivial_and_get_harder() {
    assert!(LEVELS[0].pushes <= 3);
    for pair in LEVELS.windows(2) {
        assert!(pair[0].pushes <= pair[1].pushes, "the pushes go down after {:?}", pair[0].text);
    }
    assert!(LEVELS[LEVELS.len() - 1].pushes >= 30);
    let levels = levels();
    for (i, a) in levels.iter().enumerate() {
        assert!(!levels[..i].contains(a), "level {} is there twice", i + 1);
    }
}

#[test]
#[ignore = "a tool: prints the solver's solution of every level"]
fn print_solutions() {
    for (i, level) in levels().iter().enumerate() {
        match solve(level) {
            Some((pushes, steps)) => println!("    \"{steps}\", // {}: {pushes} pushes", i + 1),
            None => println!("    // {}: NO SOLUTION", i + 1),
        }
    }
}

// The level maker.

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 11) as usize % n
    }
}

/// A random room of the given size: walls around, and pieces of wall
/// inside (more of them the higher `density` is).  `None` if the floor
/// fell apart or has a dead end.
fn room(rng: &mut Rng, w: usize, h: usize, density: usize) -> Option<Vec<bool>> {
    let mut wall = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            wall[y * w + x] = x == 0 || y == 0 || x == w - 1 || y == h - 1;
        }
    }
    let interior = (w - 2) * (h - 2);
    for _ in 0..1 + rng.below(interior * density / 20 + 1) {
        let (x, y) = (1 + rng.below(w - 2), 1 + rng.below(h - 2));
        wall[y * w + x] = true;
        // Some pieces are two or three squares in a row.
        let (dx, dy) = if rng.below(2) == 0 { (1, 0) } else { (0, 1) };
        for k in 1..=rng.below(3) {
            let (x, y) = (x + dx * k, y + dy * k);
            if x < w - 1 && y < h - 1 {
                wall[y * w + x] = true;
            }
        }
    }
    let floor: Vec<usize> = (0..w * h).filter(|i| !wall[*i]).collect();
    if floor.len() < 8 {
        return None;
    }
    let near = |i: usize| [i - w, i + w, i - 1, i + 1];
    // One piece of floor, and no square with three walls around it.
    let mut seen = vec![false; w * h];
    let mut stack = vec![floor[0]];
    seen[floor[0]] = true;
    while let Some(i) = stack.pop() {
        for j in near(i) {
            if !wall[j] && !seen[j] {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    if floor.iter().any(|i| !seen[*i] || near(*i).iter().filter(|j| wall[**j]).count() >= 3) {
        return None;
    }
    fills(&wall, w).then_some(wall)
}

/// The level's text; wall that touches no floor, even at a corner, is
/// written as outside.
fn text(w: usize, wall: &[bool], targets: &[bool], boxes: &[bool], player: usize) -> String {
    let h = wall.len() / w;
    let mut out = String::new();
    for y in 0..h {
        let mut row = String::new();
        for x in 0..w {
            let i = y * w + x;
            let touches = (y.saturating_sub(1)..(y + 2).min(h))
                .any(|ny| (x.saturating_sub(1)..(x + 2).min(w)).any(|nx| !wall[ny * w + nx]));
            row.push(match (wall[i], boxes[i], targets[i], i == player) {
                (true, ..) => {
                    if touches {
                        '#'
                    } else {
                        ' '
                    }
                }
                (_, true, true, _) => 'O',
                (_, true, false, _) => 'o',
                (_, _, true, true) => 'P',
                (_, _, false, true) => 'p',
                (_, _, true, false) => 'x',
                _ => '.',
            });
        }
        out += row.trim_end();
        out.push('\n');
    }
    out
}

/// For each floor square, the fewest pushes that bring a box from there
/// to a target if it is the only box.
fn alone(map: &Map) -> Vec<u32> {
    let mut best = vec![u32::MAX; map.cell.len()];
    let mut seen: HashMap<State, u32> = HashMap::new();
    let mut queue = VecDeque::new();
    for b in (0..map.cell.len() as u8).filter(|b| map.targets >> b & 1 == 1) {
        for at in (0..map.cell.len() as u8).filter(|at| at != &b) {
            let state: State = (1 << b, map.reach(1 << b, at).trailing_zeros() as u8);
            if seen.insert(state, 0).is_none() {
                queue.push_back(state);
            }
        }
    }
    while let Some(state) = queue.pop_front() {
        let (boxes, at) = state;
        let d = seen[&state];
        let b = boxes.trailing_zeros() as usize;
        best[b] = best[b].min(d);
        let reach = map.reach(boxes, at);
        for dir in 0..4 {
            let stand = map.adj[b][dir];
            if stand == NONE || reach >> stand & 1 == 0 {
                continue;
            }
            let to = map.adj[stand as usize][dir];
            if to == NONE {
                continue;
            }
            let next: State = (1 << stand, map.reach(1 << stand, to).trailing_zeros() as u8);
            if let Entry::Vacant(e) = seen.entry(next) {
                e.insert(d + 1);
                queue.push_back(next);
            }
        }
    }
    best
}

/// A level made from a room.
struct Made {
    /// The fewest pushes that solve it.
    pushes: u32,
    /// How many of those are more than each box would need if it were the
    /// only one: what the boxes cost by being in each other's way.
    extra: u32,
    text: String,
}

/// The position of a room with these targets that is hardest to solve,
/// as a level: the most pushes, with pushes that come from the boxes
/// being in each other's way counted four times.  Plays backward from the
/// solved positions, pulling boxes, breadth-first, so the number of pulls
/// to a position is the fewest pushes that solve it.  `None` if the room
/// has more positions than `limit`.
fn hardest(rng: &mut Rng, w: usize, wall: &[bool], targets: &[bool], limit: usize) -> Option<Made> {
    let n = wall.len();
    let first = (0..n).find(|i| !wall[*i])?;
    let solved = Level {
        width: w,
        height: n / w,
        walls: wall.to_vec(),
        targets: targets.to_vec(),
        boxes: targets.to_vec(),
        player: first,
    };
    let map = Map::new(&solved);
    let mut depth: HashMap<State, u32> = HashMap::new();
    let mut queue = VecDeque::new();
    for at in 0..map.cell.len() as u8 {
        if map.targets >> at & 1 == 0 {
            let state: State = (map.targets, map.reach(map.targets, at).trailing_zeros() as u8);
            if depth.insert(state, 0).is_none() {
                queue.push_back(state);
            }
        }
    }
    let alone = alone(&map);
    let mut far: Vec<State> = Vec::new();
    let (mut far_score, mut far_depth, mut far_extra) = (0, 0, 0);
    while let Some(state) = queue.pop_front() {
        let (boxes, at) = state;
        let d = depth[&state];
        // Among the farthest positions, those with no box at home yet.
        let home = (boxes & map.targets).count_ones();
        if home == 0 {
            let direct: u32 = (0..map.cell.len()).filter(|i| boxes >> i & 1 == 1).map(|i| alone[i]).sum();
            let score = d + 3 * (d - direct);
            if score > far_score {
                far.clear();
                (far_score, far_depth, far_extra) = (score, d, d - direct);
            }
            if score == far_score {
                far.push(state);
            }
        }
        let reach = map.reach(boxes, at);
        let mut left = boxes;
        while left != 0 {
            let b = left.trailing_zeros() as u8;
            left &= left - 1;
            for dir in 0..4 {
                // The figure stands beside the box and steps away from it.
                let stand = map.adj[b as usize][dir];
                if stand == NONE || reach >> stand & 1 == 0 {
                    continue;
                }
                let to = map.adj[stand as usize][dir];
                if to == NONE || boxes >> to & 1 == 1 {
                    continue;
                }
                let moved = boxes & !(1 << b) | 1 << stand;
                let next: State = (moved, map.reach(moved, to).trailing_zeros() as u8);
                if !depth.contains_key(&next) {
                    if depth.len() >= limit {
                        return None;
                    }
                    depth.insert(next, d + 1);
                    queue.push_back(next);
                }
            }
        }
    }
    if far_score == 0 {
        return None;
    }
    let (boxes, at) = far[rng.below(far.len())];
    let region: Vec<u8> = (0..map.cell.len() as u8).filter(|i| map.reach(boxes, at) >> i & 1 == 1).collect();
    let player = map.cell[region[rng.below(region.len())] as usize];
    let mut flags = vec![false; n];
    for (i, c) in map.cell.iter().enumerate() {
        flags[*c] = boxes >> i & 1 == 1;
    }
    Some(Made {
        pushes: far_depth,
        extra: far_extra,
        text: text(w, wall, targets, &flags, player),
    })
}

/// Makes levels: `BOXES`, `WIDTH`, `HEIGHT` (walls included), `DENSITY`
/// (how much wall inside the room), `ROOMS` (how many random rooms to
/// try), `SEED`, and `KEEP` (how many of the hardest to print) come from
/// the environment.
#[test]
#[ignore = "a tool: makes levels"]
fn make_levels() {
    let var = |name: &str, default: usize| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
    let (boxes, w, h) = (var("BOXES", 2), var("WIDTH", 8), var("HEIGHT", 6));
    assert!(w <= MAX_WIDTH && h <= MAX_HEIGHT);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (var("SEED", 1) as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
    let density = var("DENSITY", 4);
    let mut best: Vec<Made> = Vec::new();
    for _ in 0..var("ROOMS", 2000) {
        let Some(wall) = room(&mut rng, w, h, density) else { continue };
        let floor: Vec<usize> = (0..w * h).filter(|i| !wall[*i]).collect();
        if floor.len() < boxes * 3 {
            continue;
        }
        // Targets: anywhere, or side by side.
        let mut targets = vec![false; w * h];
        let together = rng.below(2) == 0;
        let mut placed: Vec<usize> = Vec::new();
        for _ in 0..200 {
            if placed.len() == boxes {
                break;
            }
            let c = floor[rng.below(floor.len())];
            let beside = placed.iter().any(|p| [p - w, p + w, p - 1, p + 1].contains(&c));
            if !targets[c] && (placed.is_empty() || !together || beside) {
                targets[c] = true;
                placed.push(c);
            }
        }
        if placed.len() < boxes {
            continue;
        }
        if let Some(found) = hardest(&mut rng, w, &wall, &targets, 3_000_000) {
            best.push(found);
            best.sort_by_key(|m| std::cmp::Reverse(m.pushes + 3 * m.extra));
            best.dedup_by(|a, b| a.text == b.text);
            best.truncate(var("KEEP", 6));
        }
    }
    for made in best {
        let solved = solve(&Level::parse(&made.text).unwrap()).expect("a made level has a solution");
        assert_eq!(solved.0, made.pushes, "the two searches disagree");
        println!("{} pushes ({} extra), {} steps\n{}", made.pushes, made.extra, solved.1.len(), made.text);
    }
}
