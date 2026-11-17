//! The rules of play: lighting, the kinds of error, the marks, and
//! when the puzzle counts as solved.

use lightup_game::{Act, Game, Grid};

fn grid(text: &str) -> Grid {
    Grid::parse(text).expect("a grid")
}

/// The lit cells of a grid, as rows of `*` (lit) and `-` (not, or black).
fn lit(g: &Grid) -> String {
    let mut out = String::new();
    for i in 0..g.cells() {
        out.push(if g.is_lit(i) { '*' } else { '-' });
        if (i + 1) % g.width() == 0 {
            out.push('\n');
        }
    }
    out
}

#[test]
fn a_light_shines_along_its_row_and_column_up_to_a_black_cell() {
    let mut g = grid(
        ".....\n\
         ..#..\n\
         .....\n\
         #....\n\
         .....",
    );
    assert_eq!(g.dark_cells(), 23);
    assert!(g.place_light(g.index(2, 2)));
    assert_eq!(lit(&g), "-----\n-----\n*****\n--*--\n--*--\n");
    assert_eq!(g.dark_cells(), 23 - 7);
    // A second light: cells both shine on stay lit when one goes.
    assert!(g.place_light(g.index(4, 4)));
    assert_eq!(lit(&g), "----*\n----*\n*****\n--*-*\n*****\n");
    assert!(g.remove_light(g.index(2, 2)));
    assert_eq!(lit(&g), "----*\n----*\n----*\n----*\n*****\n");
    assert!(g.remove_light(g.index(4, 4)));
    assert_eq!(g.dark_cells(), 23);
    // No light on a black cell, and no second light on a cell.
    assert!(!g.place_light(g.index(2, 1)));
    assert!(g.place_light(0));
    assert!(!g.place_light(0));
    assert!(!g.remove_light(1));
    // The text form and back.
    assert_eq!(Grid::parse(&g.to_text()).unwrap(), g);
    assert_eq!(g.to_text(), "L....\n..#..\n.....\n#....\n.....\n");
}

#[test]
fn lights_that_see_each_other_and_overfull_numbers_are_errors() {
    let mut g = grid(
        "..#.\n\
         .1..\n\
         ....",
    );
    let at = |g: &Grid, x, y| g.index(x, y);
    g.place_light(at(&g, 0, 0));
    assert!(!g.light_conflicts(0));
    // Across the black cell: no conflict.
    g.place_light(at(&g, 3, 0));
    assert!(!g.light_conflicts(at(&g, 0, 0)) && !g.light_conflicts(at(&g, 3, 0)));
    // In the same row with nothing between: both are in error.
    g.place_light(at(&g, 1, 0));
    assert!(g.light_conflicts(at(&g, 0, 0)) && g.light_conflicts(at(&g, 1, 0)));
    assert!(!g.light_conflicts(at(&g, 3, 0)));
    // The 1 has one light beside it: fine.  A second one is too many.
    let one = at(&g, 1, 1);
    assert_eq!(g.lights_beside(one), 1);
    assert!(!g.clue_overfull(one));
    g.place_light(at(&g, 0, 1));
    assert!(g.clue_overfull(one));
    g.remove_light(at(&g, 1, 0));
    assert!(!g.clue_overfull(one));
    // Too few lights is no error while there are cells left for them,
    // and an unnumbered black cell never is one.
    g.remove_light(at(&g, 0, 1));
    assert!(!g.clue_overfull(one) && !g.clue_unsatisfiable(one));
    assert!(!g.clue_overfull(at(&g, 2, 0)));
    assert!(!g.is_solved());
}

#[test]
fn a_number_that_can_no_longer_be_met_is_unsatisfiable() {
    let at = |g: &Grid, x, y| g.index(x, y);
    // A 2 with four white cells beside it, and a 1 in a corner.
    let mut g = grid(
        ".....\n\
         ..2..\n\
         .....\n\
         ....#\n\
         ...#1",
    );
    let two = at(&g, 2, 1);
    assert!(!g.clue_unsatisfiable(two));
    // The 1 has no white cell beside it at all.
    assert!(g.clue_unsatisfiable(at(&g, 4, 4)));
    // Marks: two of the four cells are left, which is enough; with one
    // left, it is not.
    assert!(g.set_mark(at(&g, 2, 0), true) && g.set_mark(at(&g, 1, 1), true));
    assert!(!g.clue_unsatisfiable(two));
    g.set_mark(at(&g, 3, 1), true);
    assert!(g.clue_unsatisfiable(two));
    g.set_mark(at(&g, 3, 1), false);
    assert!(!g.clue_unsatisfiable(two));
    // A light that shines on a cell beside the number, from away: that
    // cell can take no light any more, so one cell is left for two
    // lights.
    g.place_light(at(&g, 3, 3));
    assert!(g.is_lit(at(&g, 3, 1)) && !g.has_light(at(&g, 3, 1)));
    assert!(g.clue_unsatisfiable(two));
    assert!(!g.clue_overfull(two));
    g.remove_light(at(&g, 3, 3));
    assert!(!g.clue_unsatisfiable(two));
    // A light beside the number counts as a light, though its cell is
    // lit: one light and one cell left for the other.
    g.place_light(at(&g, 3, 1));
    assert_eq!(g.lights_beside(two), 1);
    assert!(!g.clue_unsatisfiable(two));
    // The second light, below (the first does not shine there): met.
    g.place_light(at(&g, 2, 2));
    assert_eq!(g.lights_beside(two), 2);
    assert!(!g.clue_unsatisfiable(two));
    // A third light, on a marked cell (the light takes the mark): too
    // many.
    g.place_light(at(&g, 2, 0));
    assert!(g.clue_overfull(two) && g.clue_unsatisfiable(two));
    g.remove_light(at(&g, 2, 0));
    assert!(!g.clue_unsatisfiable(two));
    // One light and no cell left for the second: the cell above is
    // marked, and the one on the left is marked and lit.
    g.remove_light(at(&g, 2, 2));
    g.set_mark(at(&g, 2, 0), true);
    g.place_light(at(&g, 2, 4));
    assert!(g.is_lit(at(&g, 2, 2)));
    assert!(g.clue_unsatisfiable(two) && !g.clue_overfull(two));

    // A 0 is unsatisfiable only with a light beside it, and a black cell
    // without a number never is.
    let mut g = grid("x0.\n.#.\n...");
    assert!(!g.clue_unsatisfiable(1) && !g.clue_unsatisfiable(4));
    g.place_light(2);
    assert!(g.clue_unsatisfiable(1) && !g.clue_unsatisfiable(4));

    // No number of a solved puzzle is unsatisfiable.
    let g = grid("#1L\nL..\n.#0");
    assert!(g.is_solved());
    assert!((0..9).all(|i| !g.clue_unsatisfiable(i)));
}

#[test]
fn solved_means_all_lit_no_conflict_and_every_number_met() {
    let puzzle = "#1.\n...\n.#0";
    assert!(!grid(puzzle).is_solved());
    // No conflict and no number exceeded, but a cell is dark and the 1
    // has no light.
    assert!(!grid("#1.\nL..\n.#0").is_solved());
    assert_eq!(grid("#1.\nL..\n.#0").dark_cells(), 1);
    // All lit and the 1 met, but two lights see each other and one is
    // beside the 0.
    assert!(!grid("#1L\nL.L\n.#0").is_solved());
    // All lit, no conflict, but the 1 has two lights.
    assert!(!grid("#1L\n.L.\nL#0").is_solved());
    assert_eq!(grid("#1L\n.L.\nL#0").dark_cells(), 0);
    // All lit, no conflict, the 0 met, but the 1 has no light.
    assert!(!grid("#..\nL..\n.#0").is_solved());
    let none = grid("#1x\n.Lx\nL#0");
    assert_eq!(none.dark_cells(), 1);
    assert!(grid("#1L\nL..\n.#0").is_solved());
    // Marks do not matter.
    assert!(grid("#1L\nLxx\nx#0").is_solved());
}

#[test]
fn the_cursor_and_the_three_acts() {
    let mut game = Game::new(grid("#1.\n...\n.#0"));
    assert_eq!(game.cursor(), (0, 0));
    // The edges stop the cursor.
    assert!(!game.move_cursor(-1, 0));
    assert!(!game.move_cursor(0, -1));
    // Nothing happens on a black cell, numbered or not.
    for act in [Act::Light, Act::Mark, Act::Clear] {
        assert!(!game.act(act));
        assert!(game.move_cursor(1, 0));
        assert!(!game.act(act));
        assert!(game.move_cursor(-1, 0));
    }
    assert_eq!(game.grid().to_text(), "#1.\n...\n.#0\n");

    assert!(game.move_cursor(0, 1));
    assert!(game.act(Act::Light));
    // A light is not a switch: once more changes nothing.
    assert!(!game.act(Act::Light));
    assert_eq!(game.grid().to_text(), "#1.\nL..\n.#0\n");
    // A mark takes the light's place, and a light the mark's.
    assert!(game.act(Act::Mark));
    assert!(!game.act(Act::Mark));
    assert_eq!(game.grid().to_text(), "#1.\nx..\n.#0\n");
    assert_eq!(game.grid().dark_cells(), 5);
    assert!(game.act(Act::Light));
    assert_eq!(game.grid().to_text(), "#1.\nL..\n.#0\n");
    // Clear takes off a light or a mark, and nothing from an empty cell.
    assert!(game.act(Act::Clear));
    assert!(!game.act(Act::Clear));
    assert!(game.act(Act::Mark));
    assert!(game.act(Act::Clear));
    assert_eq!(game.grid().to_text(), "#1.\n...\n.#0\n");
    // The key of the light: a light on an empty cell, off a cell with a
    // light, and in place of a mark.  The key of the mark likewise.
    assert!(game.toggle(Act::Light));
    assert_eq!(game.grid().to_text(), "#1.\nL..\n.#0\n");
    assert!(game.toggle(Act::Light));
    assert_eq!(game.grid().to_text(), "#1.\n...\n.#0\n");
    assert!(game.toggle(Act::Mark));
    assert_eq!(game.grid().to_text(), "#1.\nx..\n.#0\n");
    assert!(game.toggle(Act::Light));
    assert_eq!(game.grid().to_text(), "#1.\nL..\n.#0\n");
    assert!(game.toggle(Act::Mark));
    assert_eq!(game.grid().to_text(), "#1.\nx..\n.#0\n");
    assert!(game.toggle(Act::Mark));
    assert_eq!(game.grid().to_text(), "#1.\n...\n.#0\n");
    assert!(!game.toggle(Act::Clear));
    // Each is one change to take back.
    assert!(game.toggle(Act::Mark) && game.toggle(Act::Light) && game.undo());
    assert_eq!(game.grid().to_text(), "#1.\nx..\n.#0\n");
    assert!(game.undo());
    assert_eq!(game.grid().to_text(), "#1.\n...\n.#0\n");
    // On a black cell, nothing.
    game.set_cursor(0, 0);
    assert!(!game.toggle(Act::Light) && !game.toggle(Act::Mark));
    game.set_cursor(0, 1);

    // Solving it: the last light ends the game, and then nothing moves.
    assert!(game.act(Act::Light));
    assert!(!game.solved());
    game.set_cursor(2, 0);
    assert!(game.act(Act::Light));
    assert!(game.solved());
    assert!(!game.act(Act::Clear));
    assert!(!game.move_cursor(0, 1));
    assert_eq!(game.cursor(), (2, 0));
    assert_eq!(game.grid().to_text(), "#1L\nL..\n.#0\n");
    // A game that starts on a solved grid is solved.
    assert!(Game::new(grid("#1L\nL..\n.#0")).solved());
    // Outside the grid is its nearest cell.
    let mut game = Game::new(grid("...\n..."));
    game.set_cursor(9, 9);
    assert_eq!(game.cursor(), (2, 1));
}

#[test]
fn auto_does_what_the_numbers_settle_in_one_pass() {
    // A 0 and a 2 that share a cell, a 1 that has its light, and a 3
    // with four cells for three lights.
    let mut game = Game::new(grid(
        ".0...#.\n\
         .....1L\n\
         #2#....\n\
         ...3...\n\
         ..#.#..\n\
         .......\n\
         .......",
    ));
    game.set_cursor(6, 6);
    let before = game.clone();
    assert!(game.auto());
    // Marks beside the 0 and beside the 1 that has its light, and a
    // light below the 2, which has two cells for its two lights.  The
    // cell between the 0 and the 2, which the one wants marked and the
    // other lit, is left alone.  Nothing beside the 3.
    assert_eq!(
        game.grid().to_text(),
        "x0x..#.\n\
         ....x1L\n\
         #2#..x.\n\
         .L.3...\n\
         ..#.#..\n\
         .......\n\
         .......\n"
    );
    // The second press works on what the first left: the light below
    // the 2 shines on the cell left of the 3, so the 3 has three cells
    // for three lights.  One pass did not go that far.
    assert!(game.auto());
    assert_eq!(
        game.grid().to_text(),
        "x0x..#.\n\
         ....x1L\n\
         #2#L.x.\n\
         .L.3L..\n\
         ..#L#..\n\
         .......\n\
         .......\n"
    );
    // Nothing more to do: nothing changes, and it is no step.
    let twice = game.clone();
    assert!(!game.auto());
    assert_eq!(game, twice);
    // Kept as text and read again, a press is still one step.
    let mut read = Game::new(game.grid().clone());
    read.set_changes(&game.changes_text());
    read.set_cursor(6, 6);
    assert_eq!(read, game);
    // Undo takes a whole press back at once.
    assert!(read.undo());
    assert!(read.undo());
    assert_eq!(read, before);
    assert!(!read.undo());
    for nonsense in ["+4.", "4.++5.", "4.+", "4.+x"] {
        read.set_changes(nonsense);
        assert!(!read.can_undo(), "{nonsense}");
    }

    // A change by hand before a press is a step of its own, with the
    // cursor on its cell.
    let mut game = Game::new(grid(".0.\n...\n..."));
    game.set_cursor(2, 2);
    assert!(game.act(Act::Mark));
    game.set_cursor(0, 2);
    assert!(game.auto());
    assert_eq!(game.grid().to_text(), "x0x\n.x.\n..x\n");
    assert!(game.undo());
    assert_eq!(game.grid().to_text(), ".0.\n...\n..x\n");
    assert_eq!(game.cursor(), (0, 2));
    assert!(game.undo());
    assert_eq!(game.cursor(), (2, 2));

    // Every number is judged on the board as it was: the lights of two
    // numbers may shine on each other, and are then errors to see.
    let mut game = Game::new(grid("1..1"));
    assert!(game.auto());
    assert_eq!(game.grid().to_text(), "1LL1\n");
    assert!(game.grid().light_conflicts(1) && game.grid().light_conflicts(2));
    // A number with too many lights gets marks too, and one with too few
    // cells left gets nothing.
    let mut game = Game::new(grid("L1L\n...\n#3#\nx.."));
    assert!(game.auto());
    assert_eq!(game.grid().to_text(), "L1L\n.x.\n#3#\nx..\n");
    assert!(!game.auto());
    // The last lights solve the puzzle, and then nothing moves.
    let mut game = Game::new(grid("#2.\n...\n##."));
    assert!(game.auto() && game.solved());
    assert_eq!(game.grid().to_text(), "#2L\n.L.\n##.\n");
    assert!(!game.auto() && !game.undo());
}

#[test]
fn what_is_not_a_grid() {
    for text in ["", "..\n...", "..?\n...", ".5.", &".".repeat(15)] {
        assert!(Grid::parse(text).is_none(), "{text:?}");
    }
    assert_eq!(
        Grid::parse(" . # 3 \n\n L x . \n").unwrap().to_text(),
        ".#3\nLx.\n"
    );
}

#[test]
fn changes_are_taken_back_one_by_one() {
    use lightup_game::UNDO_STEPS;
    let fresh = "#1.\n...\n.#0\n";
    let mut game = Game::new(grid(fresh));
    // Nothing to take back at first, and moving the cursor is no change.
    assert!(!game.can_undo() && !game.undo());
    game.set_cursor(0, 1);
    assert!(game.move_cursor(1, 0));
    assert!(!game.undo());
    assert_eq!(game.cursor(), (1, 1));

    // A light, a mark in its place, a light again, cleared; a mark on
    // another cell; and keys that change nothing, which are no steps.
    assert!(game.act(Act::Light));
    assert!(!game.act(Act::Light));
    assert!(game.act(Act::Mark));
    assert!(game.act(Act::Light));
    assert!(game.act(Act::Clear));
    assert!(!game.act(Act::Clear));
    game.set_cursor(2, 1);
    assert!(game.act(Act::Mark));
    game.set_cursor(0, 0);
    assert!(!game.act(Act::Light), "a black cell");
    assert_eq!(game.grid().to_text(), "#1.\n..x\n.#0\n");

    // Back through them: each step is what the board showed before, and
    // the cursor goes to the cell.
    let mut copy = game.clone();
    for (board, cursor) in [
        ("#1.\n...\n.#0\n", (2, 1)),
        ("#1.\n.L.\n.#0\n", (1, 1)),
        ("#1.\n.x.\n.#0\n", (1, 1)),
        ("#1.\n.L.\n.#0\n", (1, 1)),
        (fresh, (1, 1)),
    ] {
        assert!(game.can_undo());
        assert!(game.undo());
        assert_eq!(
            (game.grid().to_text().as_str(), game.cursor()),
            (board, cursor)
        );
    }
    assert!(!game.can_undo() && !game.undo());

    // Two lights that see each other: taking the second back takes the
    // error off both.
    game.set_cursor(0, 1);
    game.act(Act::Light);
    game.set_cursor(2, 1);
    game.act(Act::Light);
    assert!(game.grid().light_conflicts(3) && game.grid().light_conflicts(5));
    assert!(game.undo());
    assert!(!game.grid().light_conflicts(3) && !game.grid().has_light(5));

    // The changes are kept as text, and read back: the copy and a game
    // read from its text take the same steps back.
    let mut read = Game::new(copy.grid().clone());
    read.set_changes(&copy.changes_text());
    while copy.undo() {
        assert!(read.undo());
        assert_eq!(read.grid(), copy.grid());
    }
    assert!(!read.undo());
    // Text that is not that is no changes at all.
    for nonsense in ["4", "4.99L", "0L", "4?", "L", "4.4", "1x"] {
        read.set_changes(nonsense);
        assert!(!read.can_undo(), "{nonsense:?}");
    }
    read.set_changes("");
    assert!(!read.can_undo());

    // A solved puzzle takes nothing back.
    game.set_cursor(2, 0);
    assert!(game.act(Act::Light));
    assert!(game.solved());
    assert!(!game.can_undo() && !game.undo());
    assert_eq!(game.grid().to_text(), "#1L\nL..\n.#0\n");

    // The oldest changes are forgotten beyond the bound.
    let mut game = Game::new(grid(fresh));
    game.set_cursor(1, 1);
    for _ in 0..UNDO_STEPS {
        game.act(Act::Mark);
        game.act(Act::Clear);
    }
    assert!(game.changes_text().len() <= 2 * UNDO_STEPS);
    let mut steps = 0;
    while game.undo() {
        steps += 1;
    }
    assert_eq!(steps, UNDO_STEPS);
    let mut long = Game::new(grid(fresh));
    long.set_changes(&"4.4x".repeat(UNDO_STEPS));
    assert_eq!(long.changes_text().len(), 2 * UNDO_STEPS);
}
