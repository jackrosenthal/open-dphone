//! The tables of the best times, and the times that wait to be written.

use lightup_game::generate::{Difficulty, SIZES};
use lightup_game::scores::{Kind, NAME_CHARS, Pending, ROWS, Table, clock};

fn names(table: &Table) -> Vec<(&str, u64)> {
    table.rows().iter().map(|r| (r.name.as_str(), r.ms)).collect()
}

#[test]
fn there_is_a_kind_for_each_size_and_difficulty() {
    let all: Vec<Kind> = Kind::all().collect();
    assert_eq!(all.len(), Kind::COUNT);
    assert_eq!(all.len(), 9);
    for (n, kind) in all.iter().enumerate() {
        assert_eq!(kind.index(), n);
        assert_eq!(Kind::from_index(n), *kind);
        assert_eq!(Kind::named(&kind.name()), Some(*kind));
        assert_eq!(Kind::of_key(&kind.key()), Some(*kind));
    }
    let first = Kind::new(7, Difficulty::Easy);
    assert_eq!(first.title(), "7x7 Easy");
    assert_eq!(first.name(), "7x7-easy");
    assert_eq!(first.key(), "best/7x7-easy");
    let last = Kind::new(14, Difficulty::Hard);
    assert_eq!(last.title(), "14x14 Hard");
    assert_eq!(last.key(), "best/14x14-hard");
    assert_eq!(Kind::named("8x8-easy"), None);
    assert_eq!(Kind::of_key("7x7-easy"), None);
    assert_eq!(Kind::of_key("best/7x7"), None);
}

#[test]
fn a_key_is_one_the_store_takes() {
    for kind in Kind::all() {
        let key = kind.key();
        assert!((1..=64).contains(&key.len()));
        assert!(
            key.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-/:".contains(c)),
            "{key}"
        );
    }
}

#[test]
fn the_kinds_go_around() {
    let kind = Kind::new(10, Difficulty::Medium);
    assert_eq!(kind.next_size(), Kind::new(14, Difficulty::Medium));
    assert_eq!(kind.next_size().next_size(), Kind::new(7, Difficulty::Medium));
    assert_eq!(kind.next_difficulty(), Kind::new(10, Difficulty::Hard));
    assert_eq!(
        kind.next_difficulty().next_difficulty(),
        Kind::new(10, Difficulty::Easy)
    );
    assert_eq!(kind.step(1), Kind::new(10, Difficulty::Hard));
    assert_eq!(kind.step(-1), Kind::new(10, Difficulty::Easy));
    assert_eq!(
        Kind::new(SIZES[0], Difficulty::Easy).step(-1),
        Kind::new(14, Difficulty::Hard)
    );
    assert_eq!(
        Kind::new(14, Difficulty::Hard).step(1),
        Kind::new(SIZES[0], Difficulty::Easy)
    );
}

#[test]
fn a_time_is_shown_in_minutes_and_seconds() {
    assert_eq!(clock(0), "0:00");
    assert_eq!(clock(999), "0:00");
    assert_eq!(clock(1000), "0:01");
    assert_eq!(clock(185_999), "3:05");
    assert_eq!(clock(3_599_000), "59:59");
    assert_eq!(clock(3_723_000), "1:02:03");
}

#[test]
fn the_best_time_is_first_and_who_was_first_stays_first() {
    let mut t = Table::default();
    assert!(t.add(90_000, "101", "Alice"));
    assert!(t.add(60_000, "102", "Bob"));
    assert!(t.add(120_000, "103", "Carol"));
    // As good as Bob's, and later.
    assert!(t.add(60_000, "104", "Dan"));
    assert_eq!(
        names(&t),
        [("Bob", 60_000), ("Dan", 60_000), ("Alice", 90_000), ("Carol", 120_000)]
    );
    assert_eq!(t.place("101"), Some(2));
    assert_eq!(t.place("999"), None);
}

#[test]
fn a_player_is_in_a_table_once_with_their_best_time() {
    let mut t = Table::default();
    t.add(90_000, "101", "Alice");
    t.add(60_000, "102", "Bob");
    // No better: the table is as it was.
    let before = t.clone();
    assert!(!t.add(95_000, "101", "Alice"));
    assert!(!t.add(90_000, "101", "Alice"));
    assert_eq!(t, before);
    // Better: in place of the row.  The name is the one the PBX has now.
    assert!(t.add(50_000, "101", "Alice Smith"));
    assert_eq!(names(&t), [("Alice Smith", 50_000), ("Bob", 60_000)]);
    // Two players of one name are two players.
    assert!(t.add(70_000, "105", "Bob"));
    assert_eq!(t.rows().len(), 3);
}

#[test]
fn a_table_keeps_the_ten_best() {
    let mut t = Table::default();
    for n in 0..ROWS as u64 {
        assert!(t.add(10_000 * (n + 1), &format!("{}", 100 + n), &format!("P{n}")));
    }
    assert_eq!(t.rows().len(), 10);
    // Not among the best, also when as good as the last: nothing changes.
    let before = t.clone();
    assert!(!t.add(100_000, "200", "Late"));
    assert!(!t.add(500_000, "200", "Slow"));
    assert_eq!(t, before);
    // Among them: the last one leaves.
    assert!(t.add(15_000, "200", "Quick"));
    assert_eq!(t.rows().len(), 10);
    assert_eq!(t.rows()[1].name, "Quick");
    assert_eq!(t.place("109"), None);
    // The last player's better time takes the place of their row.
    assert!(t.add(5_000, "108", "P8"));
    assert_eq!(t.rows().len(), 10);
    assert_eq!(t.rows()[0].name, "P8");
    assert_eq!(t.rows()[9].name, "P7");
}

#[test]
fn a_table_is_text_and_reads_back_as_it_was() {
    let mut t = Table::default();
    t.add(90_000, "101", "Alice");
    t.add(61_234, "102", "Bob O'Neil, Jr.");
    let bytes = t.to_bytes();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        "61234\t102\tBob O'Neil, Jr.\n90000\t101\tAlice\n"
    );
    assert_eq!(Table::parse(&bytes), t);
    assert_eq!(Table::default().to_bytes(), b"");
    assert_eq!(Table::parse(b""), Table::default());
}

#[test]
fn a_name_cannot_break_a_table() {
    let mut t = Table::default();
    t.add(1_000, "101", "  Al\tice\nSmith ");
    t.add(2_000, "1\t02", &"x".repeat(100));
    assert_eq!(t.rows()[0].name, "Al ice Smith");
    assert_eq!(t.rows()[1].id, "1 02");
    assert_eq!(t.rows()[1].name.chars().count(), NAME_CHARS);
    assert_eq!(Table::parse(&t.to_bytes()), t);
    // Ten rows of the longest names, in letters of four bytes, with long
    // ids, are within a value of the store (4096 bytes).
    let mut t = Table::default();
    for n in 0..ROWS as u64 {
        let id = format!("{n}{}", "i".repeat(63));
        t.add(u64::MAX - n, &id, &"\u{1F4A1}".repeat(100));
    }
    assert_eq!(t.rows().len(), ROWS);
    assert!(t.to_bytes().len() <= 4096, "{}", t.to_bytes().len());
}

#[test]
fn what_is_not_a_table_is_read_as_what_it_holds_of_one() {
    // Not rows, a second row of a player, out of order, and too many.
    let t = Table::parse(
        b"nonsense\n\n70000\t103\tCarol\nfast\t104\tDan\n60000\t101\tAlice\n\
          65000\t101\tAlice\n80000\t105\n\xff\xfe\n90000\t106\tEve\twith more\n",
    );
    assert_eq!(names(&t), [("Alice", 60_000), ("Carol", 70_000), ("Eve", 90_000)]);
    let mut many = String::new();
    for n in 0..30 {
        many.push_str(&format!("{}\t{n}\tP{n}\n", 1000 + n));
    }
    assert_eq!(Table::parse(many.as_bytes()).rows().len(), ROWS);
}

#[test]
fn the_times_that_wait_are_the_best_of_each_kind() {
    let easy = Kind::new(7, Difficulty::Easy);
    let hard = Kind::new(14, Difficulty::Hard);
    let mut p = Pending::default();
    assert!(p.is_empty());
    assert!(p.add(easy, 90_000));
    assert!(p.add(hard, 600_000));
    assert!(!p.add(easy, 95_000));
    assert!(p.add(easy, 80_000));
    assert_eq!(p.times(), [(easy, 80_000), (hard, 600_000)]);
    assert_eq!(p.of(easy), Some(80_000));
    assert_eq!(p.of(Kind::new(10, Difficulty::Easy)), None);

    assert_eq!(p.to_text(), "7x7-easy 80000\n14x14-hard 600000\n");
    assert_eq!(Pending::parse(&p.to_text()), p);
    assert_eq!(
        Pending::parse("7x7-easy 80000\nnonsense\n9x9-easy 5\n7x7-hard soon\n14x14-hard 600000\n"),
        p
    );
    assert!(Pending::parse("").is_empty());
}

#[test]
fn a_written_time_waits_no_longer_but_a_better_one_that_came_meanwhile_does() {
    let easy = Kind::new(7, Difficulty::Easy);
    let mut p = Pending::default();
    p.add(easy, 90_000);
    // While 90 seconds are being written, the player does better.
    p.add(easy, 70_000);
    assert!(!p.done(easy, 90_000));
    assert_eq!(p.of(easy), Some(70_000));
    assert!(p.done(easy, 70_000));
    assert!(p.is_empty());
    assert!(!p.done(easy, 70_000));
}
