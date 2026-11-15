//! The solver against a plain search that knows nothing of the deduction
//! rules, and on grids small enough to check by hand.

use lightup_game::Grid;
use lightup_game::rng::Rng;
use lightup_game::solver::{Outcome, Solve, Techniques, solve};

fn grid(text: &str) -> Grid {
    Grid::parse(text).expect("a grid")
}

/// Counts solutions by trying a light and no light on every white cell in
/// turn (no light only, on a marked cell), checking the rules only at the
/// end (and stopping at `limit`).
fn brute(g: &mut Grid, from: usize, limit: usize) -> usize {
    let Some(i) = (from..g.cells()).find(|i| !g.is_black(*i)) else {
        return g.is_solved() as usize;
    };
    let mut n = brute(g, i + 1, limit);
    if n < limit && !g.is_lit(i) && !g.is_marked(i) {
        g.place_light(i);
        n += brute(g, i + 1, limit - n);
        g.remove_light(i);
    }
    n
}

#[test]
fn none_one_and_several_solutions() {
    // A white cell beside a 0: nothing can light it.
    assert_eq!(solve(&grid(".0"), Techniques::EASY), Outcome::None);
    // Beside a 1: the light goes there.
    assert_eq!(solve(&grid(".1"), Techniques::EASY), Outcome::Unique);
    // Four white cells: either diagonal.  The rules alone cannot choose;
    // with guessing both are found.
    let open = grid("..\n..");
    assert_eq!(solve(&open, Techniques::EASY), Outcome::Undetermined);
    assert_eq!(solve(&open, Techniques::MEDIUM), Outcome::Undetermined);
    assert_eq!(solve(&open, Techniques::HARD), Outcome::Several);
    assert_eq!(brute(&mut open.clone(), 0, 9), 2);
    // An open 3x3 grid has six solutions (one light in each row and
    // column).
    assert_eq!(brute(&mut grid("...\n...\n..."), 0, 9), 6);
    assert_eq!(
        solve(&grid("...\n...\n..."), Techniques::HARD),
        Outcome::Several
    );
    // A 1 in the corner of a 2x2 grid: its light lights one more cell,
    // and the last cell's own light would be the clue's second.
    assert_eq!(solve(&grid("1.\n.."), Techniques::HARD), Outcome::None);
    assert_eq!(brute(&mut grid("1.\n.."), 0, 9), 0);
    // Lights that see each other are no solution, whatever else holds.
    assert_eq!(solve(&grid("L.L"), Techniques::HARD), Outcome::None);
    // A solved grid is its own one solution.
    assert_eq!(solve(&grid("L.\n.L"), Techniques::EASY), Outcome::Unique);
    // Marks are taken as given: with the other diagonal ruled out, one
    // solution is left; with a whole row ruled out, none.
    assert_eq!(solve(&grid("x.\n.."), Techniques::MEDIUM), Outcome::Unique);
    assert_eq!(solve(&grid("xx\n.."), Techniques::HARD), Outcome::None);
}

#[test]
fn the_solved_grid_is_left_behind() {
    let mut s = Solve::new(&grid("..\n.."), Techniques::HARD);
    assert_eq!(s.run(), Outcome::Several);
    assert_eq!(s.deepest_guess(), 1);
    assert!(s.solution().unwrap().is_solved());
    // Four cells around a 2, none seeing another: each needs its own
    // light, which is two too many.
    assert_eq!(
        solve(&grid("#.#\n.2.\n#.#"), Techniques::HARD),
        Outcome::None
    );
    let mut s = Solve::new(&grid("#.#\n.4.\n#.#"), Techniques::EASY);
    assert_eq!(s.run(), Outcome::Unique);
    assert_eq!(s.solution().unwrap().to_text(), "#L#\nL4L\n#L#\n");
    assert_eq!(s.deepest_guess(), 0);
}

#[test]
fn rule_a_a_dark_cell_with_one_source() {
    // The marked cell cannot light itself; only its neighbor can.
    let mut s = Solve::new(&grid("###\nx.#\n###"), Techniques::EASY);
    assert_eq!(s.run(), Outcome::Unique);
    assert_eq!(s.solution().unwrap().to_text(), "###\nxL#\n###\n");
}

#[test]
fn rule_b_full_and_exactly_fillable_clues() {
    // A 2 in the corner takes both cells beside it; the last cell then
    // needs its own light, which nothing forbids.
    let mut s = Solve::new(&grid("2.#\n.#.\n###"), Techniques::EASY);
    assert_eq!(s.run(), Outcome::Unique);
    assert_eq!(s.solution().unwrap().to_text(), "2L#\nL#L\n###\n");
    // A full 1 excludes its other cells, which then must be lit from
    // elsewhere: here they cannot be.
    assert_eq!(
        solve(&grid("#L#\n#1.\n#.#"), Techniques::EASY),
        Outcome::None
    );
    // A clue with more lights than its number, or one that can no longer
    // get enough.
    assert_eq!(solve(&grid("L1L"), Techniques::HARD), Outcome::None);
    assert_eq!(solve(&grid("x3.\n#.#"), Techniques::HARD), Outcome::None);
}

#[test]
fn rule_c_set_elimination() {
    // The corner cell sees both cells that could light the cell beside
    // the 0, so it holds no light.  Rules A and B alone are stuck.
    let a = grid("..#\n..0\n###");
    assert_eq!(solve(&a, Techniques::EASY), Outcome::Undetermined);
    let mut s = Solve::new(&a, Techniques::MEDIUM);
    assert_eq!(s.run(), Outcome::Unique);
    assert_eq!(s.solution().unwrap().to_text(), "xL#\nLx0\n###\n");
    assert_eq!(brute(&mut a.clone(), 0, 9), 1);

    // The set of a clue: one of the two cells beside the 1 gets a light,
    // and the corner cell sees both, so the corner holds none.  (Then
    // each of the two cells can only be lit by itself, which is one
    // light too many: this little grid has no solution, and it takes
    // the rule to see that without guessing.)
    let b = grid("..#\n.1#\n###");
    assert_eq!(solve(&b, Techniques::EASY), Outcome::Undetermined);
    assert_eq!(solve(&b, Techniques::MEDIUM), Outcome::None);
    assert_eq!(brute(&mut b.clone(), 0, 9), 0);

    // A kill through a shared clue.  The marked cell in the corner can
    // be lit from the two cells above it.  A light on the top middle
    // cell would light the upper of them and fill the 1, which excludes
    // the lower: so the top middle cell holds no light.
    let c = grid("...\n#1.\n##x");
    assert_eq!(solve(&c, Techniques::EASY), Outcome::Undetermined);
    let mut s = Solve::new(&c, Techniques::MEDIUM);
    assert_eq!(s.run(), Outcome::Unique);
    assert_eq!(s.solution().unwrap().to_text(), "Lxx\n#1L\n##x\n");
    assert_eq!(brute(&mut c.clone(), 0, 9), 1);
}

/// Random small grids with random clues from a random arrangement of
/// lights (so most have solutions), some clues dropped: the solver must
/// agree with the plain search whenever it claims an answer, and the
/// complete solver must always agree.
#[test]
fn agrees_with_a_plain_search() {
    let mut rng = Rng::new(7);
    let mut tally = [0usize; 4];
    for round in 0..4000 {
        let (w, h) = (1 + rng.below(5) as usize, 1 + rng.below(5) as usize);
        let mut g = Grid::new(w, h);
        for i in 0..g.cells() {
            if rng.below(100) < 25 {
                g.set_black(i);
            }
        }
        lightup_game::generate::place_lights(&mut g, &mut rng);
        for i in 0..g.cells() {
            if g.is_black(i) && rng.below(100) < 60 {
                // Now and then a wrong number.
                let wrong = (rng.below(100) < 4) as u8;
                g.set_clue(i, Some(g.lights_beside(i) as u8 + wrong));
            }
        }
        g.clear_play();
        let truth = match brute(&mut g.clone(), 0, 2) {
            0 => Outcome::None,
            1 => Outcome::Unique,
            _ => Outcome::Several,
        };
        tally[truth as usize] += 1;
        assert_eq!(
            solve(&g, Techniques::COMPLETE),
            truth,
            "round {round}: {g:?}"
        );
        for tech in [Techniques::EASY, Techniques::MEDIUM, Techniques::HARD] {
            let mut s = Solve::new(&g, tech);
            let got = s.run();
            assert!(
                got == truth || got == Outcome::Undetermined,
                "round {round} {tech:?}: {got:?} for {g:?}"
            );
            if got == Outcome::Unique {
                let solved = s.solution().unwrap();
                assert!(solved.is_solved(), "{solved:?}");
                // The solution is the puzzle with lights on it.
                let mut bare = solved.clone();
                bare.clear_play();
                assert_eq!(bare, g);
            }
        }
    }
    // All three kinds came up.
    assert!(tally[..3].iter().all(|n| *n > 100), "{tally:?}");
}

/// The same on larger grids, where only the count of the complete solver
/// can be compared with a search that stops at two.
#[test]
fn agrees_with_a_plain_search_on_7x7() {
    let mut rng = Rng::new(99);
    for round in 0..40 {
        let mut g = Grid::new(7, 7);
        for i in 0..g.cells() {
            if rng.below(100) < 30 {
                g.set_black(i);
            }
        }
        lightup_game::generate::place_lights(&mut g, &mut rng);
        for i in 0..g.cells() {
            if g.is_black(i) && rng.below(100) < 70 {
                g.set_clue(i, Some(g.lights_beside(i) as u8));
            }
        }
        g.clear_play();
        let truth = if brute(&mut g.clone(), 0, 2) == 1 {
            Outcome::Unique
        } else {
            Outcome::Several
        };
        assert_eq!(
            solve(&g, Techniques::COMPLETE),
            truth,
            "round {round}: {g:?}"
        );
    }
}
