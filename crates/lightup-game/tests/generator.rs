//! The generator: for every size and difficulty on offer, a batch of
//! seeds; every puzzle has exactly one solution and is of the difficulty
//! asked for, by the definitions in `generate.rs`.

use lightup_game::Grid;
use lightup_game::generate::{
    Difficulty, Generator, Params, Progress, SIZES, Symmetry, black_layout, generate, number_all,
    place_lights,
};
use lightup_game::rng::Rng;
use lightup_game::solver::{Outcome, Solve, Techniques, solve};

/// Counts solutions (up to `limit`) without the solver: a light or none
/// on every white cell in turn, never a light where one already shines,
/// the rules checked at the end.
fn brute(g: &mut Grid, from: usize, limit: usize) -> usize {
    let Some(i) = (from..g.cells()).find(|i| !g.is_black(*i)) else {
        return g.is_solved() as usize;
    };
    let mut n = brute(g, i + 1, limit);
    if n < limit && !g.is_lit(i) {
        g.place_light(i);
        n += brute(g, i + 1, limit - n);
        g.remove_light(i);
    }
    n
}

fn check(puzzle: &Grid, side: usize, difficulty: Difficulty, what: &str) {
    assert_eq!((puzzle.width(), puzzle.height()), (side, side), "{what}");
    assert!(
        (0..puzzle.cells()).all(|i| !puzzle.has_light(i) && !puzzle.is_marked(i)),
        "{what}"
    );
    // The techniques of the difficulty find the one solution...
    let mut s = Solve::new(puzzle, difficulty.techniques());
    assert_eq!(s.run(), Outcome::Unique, "{what}: {puzzle:?}");
    assert!(s.solution().unwrap().is_solved(), "{what}");
    // ... the solver without a bound on guessing agrees that it is the
    // only one...
    assert_eq!(
        solve(puzzle, Techniques::COMPLETE),
        Outcome::Unique,
        "{what}: {puzzle:?}"
    );
    // ... and the next easier techniques do not solve it.
    match difficulty {
        Difficulty::Easy => assert_eq!(s.deepest_guess(), 0),
        Difficulty::Medium => {
            assert_eq!(s.deepest_guess(), 0);
            assert_eq!(
                solve(puzzle, Techniques::EASY),
                Outcome::Undetermined,
                "{what}: {puzzle:?}"
            );
        }
        Difficulty::Hard => {
            assert!(
                (1..=5).contains(&s.deepest_guess()),
                "{what}: {}",
                s.deepest_guess()
            );
            assert_eq!(
                solve(puzzle, Techniques::MEDIUM),
                Outcome::Undetermined,
                "{what}: {puzzle:?}"
            );
        }
    }
    // No number can go: without any one of them the puzzle is no longer
    // solved by these techniques.
    for i in 0..puzzle.cells() {
        if puzzle.clue(i).is_some() {
            let mut less = puzzle.clone();
            less.set_clue(i, None);
            assert_ne!(
                solve(&less, difficulty.techniques()),
                Outcome::Unique,
                "{what}: number at {i}"
            );
        }
    }
}

#[test]
fn every_size_and_difficulty_gives_puzzles_with_one_solution() {
    for side in SIZES {
        let seeds = match side {
            7 => 25,
            10 => 12,
            _ => 6,
        };
        for difficulty in Difficulty::ALL {
            for seed in 0..seeds {
                let what = format!("{side}x{side} {difficulty:?} seed {seed}");
                let puzzle = generate(Params::preset(side, difficulty), seed, 1000).expect(&what);
                check(&puzzle, side, difficulty, &what);
            }
        }
    }
}

#[test]
fn a_plain_search_finds_exactly_one_solution_too() {
    for difficulty in Difficulty::ALL {
        for seed in 100..106 {
            let puzzle = generate(Params::preset(7, difficulty), seed, 1000).unwrap();
            assert_eq!(
                brute(&mut puzzle.clone(), 0, 2),
                1,
                "{difficulty:?} seed {seed}: {puzzle:?}"
            );
        }
    }
}

#[test]
fn a_seed_gives_one_puzzle_however_the_work_is_cut_up() {
    let params = Params::preset(10, Difficulty::Hard);
    let whole = generate(params, 42, 1000).unwrap();
    // A step at a time, as the app does it.
    let mut g = Generator::new(params, 42, 1000);
    let mut steps = 0;
    while g.step() == Progress::Working {
        assert!(g.puzzle().is_none());
        steps += 1;
    }
    assert_eq!(g.puzzle().unwrap(), &whole);
    assert_eq!(g.steps(), steps + 1);
    assert!(g.attempts() >= 1);
    // Done stays done.
    assert_eq!(g.step(), Progress::Done);
    // Another seed, another puzzle.
    assert_ne!(generate(params, 43, 1000).unwrap(), whole);
    assert_eq!(generate(params, 42, 1000).unwrap(), whole);
}

#[test]
fn giving_up_after_the_allowed_number_of_grids() {
    // Hard 7x7 puzzles rarely come from the first grid.
    let params = Params::preset(7, Difficulty::Hard);
    let failed = (0..40)
        .filter(|seed| {
            let mut g = Generator::new(params, *seed, 1);
            let end = loop {
                match g.step() {
                    Progress::Working => {}
                    end => break end,
                }
            };
            assert_eq!(g.attempts(), 1);
            assert_eq!(end == Progress::Failed, g.puzzle().is_none());
            assert_eq!(g.step(), end);
            end == Progress::Failed
        })
        .count();
    assert!(failed > 20, "{failed}");
}

#[test]
fn the_black_cells_follow_the_symmetry() {
    let mut rng = Rng::new(5);
    let black = |g: &Grid| (0..g.cells()).filter(|i| g.is_black(*i)).count();
    for _ in 0..50 {
        // A half turn: 14x7 cells chosen from, 19 of them black.
        let g = black_layout(&Params::preset(14, Difficulty::Easy), 20, &mut rng);
        assert_eq!(black(&g), 38);
        for i in 0..g.cells() {
            assert_eq!(g.is_black(i), g.is_black(g.cells() - 1 - i));
        }
        // 10x10: 10x5 cells, 10 black.
        assert_eq!(
            black(&black_layout(
                &Params::preset(10, Difficulty::Easy),
                20,
                &mut rng
            )),
            20
        );
        // Quarter turns of 7x7: 3x4 cells chosen from, 2 of them black,
        // and the center on its own.
        let g = black_layout(&Params::preset(7, Difficulty::Easy), 20, &mut rng);
        assert!(black(&g) == 8 || black(&g) == 9, "{g:?}");
        assert_eq!(black(&g) == 9, g.is_black(g.index(3, 3)));
        for y in 0..7 {
            for x in 0..7 {
                assert_eq!(
                    g.is_black(g.index(x, y)),
                    g.is_black(g.index(6 - y, x)),
                    "{g:?}"
                );
            }
        }
        // The other symmetries, on a grid that is not square.
        let p = |symmetry| Params {
            width: 9,
            height: 6,
            black_percent: 30,
            symmetry,
            difficulty: Difficulty::Easy,
        };
        let g = black_layout(&p(Symmetry::None), 30, &mut rng);
        assert_eq!(black(&g), 16);
        let g = black_layout(&p(Symmetry::Mirror2), 30, &mut rng);
        assert_eq!(black(&g), 16);
        let g4 = black_layout(&p(Symmetry::Mirror4), 30, &mut rng);
        for y in 0..6 {
            for x in 0..9 {
                assert_eq!(g.is_black(g.index(x, y)), g.is_black(g.index(x, 5 - y)));
                assert_eq!(g4.is_black(g4.index(x, y)), g4.is_black(g4.index(8 - x, y)));
                assert_eq!(g4.is_black(g4.index(x, y)), g4.is_black(g4.index(x, 5 - y)));
            }
        }
    }
}

#[test]
fn the_lights_placed_light_everything_and_none_sees_another() {
    let mut rng = Rng::new(6);
    for round in 0..200 {
        let side = SIZES[round % 3];
        let mut g = black_layout(
            &Params::preset(side, Difficulty::Easy),
            20 + (round % 5) as u8 * 15,
            &mut rng,
        );
        place_lights(&mut g, &mut rng);
        assert_eq!(g.dark_cells(), 0);
        assert!((0..g.cells()).all(|i| !g.light_conflicts(i)));
        // Numbered all over, the arrangement is a solution.
        number_all(&mut g);
        assert!(g.is_solved());
        assert!((0..g.cells()).all(|i| g.is_black(i) == g.clue(i).is_some()));
    }
}
