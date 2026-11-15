//! The rules of play: lighting, the two kinds of error, the marks, and
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
    // Too few lights is not shown as an error, and an unnumbered black
    // cell never is.
    g.remove_light(at(&g, 0, 1));
    assert!(!g.clue_overfull(one));
    assert!(!g.clue_overfull(at(&g, 2, 0)));
    assert!(!g.is_solved());
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
fn what_is_not_a_grid() {
    for text in ["", "..\n...", "..?\n...", ".5.", &".".repeat(15)] {
        assert!(Grid::parse(text).is_none(), "{text:?}");
    }
    assert_eq!(
        Grid::parse(" . # 3 \n\n L x . \n").unwrap().to_text(),
        ".#3\nLx.\n"
    );
}
