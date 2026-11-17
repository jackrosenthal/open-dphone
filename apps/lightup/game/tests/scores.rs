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
fn each_time_is_a_row_of_its_own() {
    let mut t = Table::default();
    t.add(90_000, "101", "Alice");
    t.add(60_000, "102", "Bob");
    // Worse than her other time, and better: each is a row beside it,
    // with the name the PBX had when it was written.
    assert!(t.add(95_000, "101", "Alice"));
    assert!(t.add(50_000, "101", "Alice Smith"));
    assert_eq!(
        names(&t),
        [("Alice Smith", 50_000), ("Bob", 60_000), ("Alice", 90_000), ("Alice", 95_000)]
    );
    // Her best row.
    assert_eq!(t.place("101"), Some(0));
    assert_eq!(t.place("102"), Some(1));
    // The very time of a row of hers is that time, written before: the
    // table is as it was.
    let before = t.clone();
    assert!(!t.add(90_000, "101", "Alice"));
    assert!(!t.add(50_000, "101", "Alice Smith"));
    assert_eq!(t, before);
    // Another player's, or another millisecond, is another time.
    assert!(t.add(90_000, "102", "Bob"));
    assert!(t.add(90_001, "101", "Alice"));
    assert_eq!(t.rows().len(), 6);
    // Two players of one name are two players.
    assert!(t.add(70_000, "105", "Bob"));
    assert_eq!(t.rows().len(), 7);
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
    // A player's better time is a row of its own: the last row leaves,
    // and the player's other row stays.
    assert!(t.add(12_000, "103", "P3"));
    assert_eq!(t.rows().len(), 10);
    assert_eq!(t.rows()[1].name, "P3");
    assert_eq!(t.rows()[5].name, "P3");
    assert_eq!(t.rows()[9].name, "P7");
    assert_eq!(t.place("103"), Some(1));
    assert_eq!(t.place("108"), None);
    // A table can be one player's.
    let mut t = Table::default();
    for n in 0..ROWS as u64 + 2 {
        assert!(t.add(60_000 - n, "101", "Alice"));
    }
    assert_eq!(t.rows().len(), 10);
    assert_eq!(t.rows()[0].ms, 60_000 - ROWS as u64 - 1);
    assert_eq!(t.rows()[9].ms, 60_000 - 2);
}

#[test]
fn a_table_is_text_and_reads_back_as_it_was() {
    let mut t = Table::default();
    t.add(90_000, "101", "Alice");
    t.add(61_234, "102", "Bob O'Neil, Jr.");
    t.add(75_500, "101", "Alice");
    let bytes = t.to_bytes();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        "61234\t102\tBob O'Neil, Jr.\n75500\t101\tAlice\n90000\t101\tAlice\n"
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
    // Not rows, a row twice, out of order, and too many.
    let t = Table::parse(
        b"nonsense\n\n70000\t103\tCarol\nfast\t104\tDan\n60000\t101\tAlice\n\
          65000\t101\tAlice\n80000\t105\n\xff\xfe\n90000\t106\tEve\twith more\n\
          60000\t101\tAlice\n",
    );
    assert_eq!(
        names(&t),
        [("Alice", 60_000), ("Alice", 65_000), ("Carol", 70_000), ("Eve", 90_000)]
    );
    let mut many = String::new();
    for n in 0..30 {
        many.push_str(&format!("{}\t{n}\tP{n}\n", 1000 + n));
    }
    assert_eq!(Table::parse(many.as_bytes()).rows().len(), ROWS);
}

#[test]
fn every_time_waits() {
    let easy = Kind::new(7, Difficulty::Easy);
    let hard = Kind::new(14, Difficulty::Hard);
    let mut p = Pending::default();
    assert!(p.is_empty());
    assert!(p.add(easy, 90_000));
    assert!(p.add(hard, 600_000));
    assert!(p.add(easy, 95_000));
    assert!(p.add(easy, 80_000));
    assert!(p.add(easy, 80_000));
    assert_eq!(
        p.times(),
        [(easy, 90_000), (hard, 600_000), (easy, 95_000), (easy, 80_000), (easy, 80_000)]
    );
    assert_eq!(p.of(easy).collect::<Vec<_>>(), [90_000, 95_000, 80_000, 80_000]);
    assert_eq!(p.of(Kind::new(10, Difficulty::Easy)).count(), 0);

    let text =
        "7x7-easy 90000\n14x14-hard 600000\n7x7-easy 95000\n7x7-easy 80000\n7x7-easy 80000\n";
    assert_eq!(p.to_text(), text);
    assert_eq!(Pending::parse(&p.to_text()), p);
    assert_eq!(
        Pending::parse(
            "7x7-easy 90000\nnonsense\n14x14-hard 600000\n9x9-easy 5\n7x7-easy 95000\n\
             7x7-hard soon\n7x7-easy 80000\n7x7-easy 80000\n"
        ),
        p
    );
    assert!(Pending::parse("").is_empty());
}

#[test]
fn of_each_kind_the_ten_best_times_wait() {
    let easy = Kind::new(7, Difficulty::Easy);
    let hard = Kind::new(14, Difficulty::Hard);
    let mut p = Pending::default();
    p.add(hard, 600_000);
    for n in 0..ROWS as u64 {
        assert!(p.add(easy, 60_000 + 1_000 * (n % 5)));
    }
    let full = p.clone();
    // Worse than each of them, or as good as the worst: it could not
    // make the table once they are written.
    assert!(!p.add(easy, 70_000));
    assert!(!p.add(easy, 64_000));
    assert_eq!(p, full);
    // Better: the worst leaves, of two equals the one that came later.
    assert!(p.add(easy, 50_000));
    let easy_times: Vec<u64> = p.of(easy).collect();
    assert_eq!(
        easy_times,
        [60_000, 61_000, 62_000, 63_000, 64_000, 60_000, 61_000, 62_000, 63_000, 50_000]
    );
    // The other kinds have room of their own.
    assert!(p.add(hard, 900_000));
    assert_eq!(p.of(hard).count(), 2);
    // So does a file with too many.
    let mut many = String::new();
    for n in 0..30 {
        many.push_str(&format!("7x7-easy {}\n", 90_000 - n));
    }
    let read = Pending::parse(&many);
    assert_eq!(read.of(easy).count(), ROWS);
    assert_eq!(read.of(easy).max(), Some(90_000 - 20));
}

#[test]
fn a_written_time_waits_no_longer_and_the_others_still_do() {
    let easy = Kind::new(7, Difficulty::Easy);
    let mut p = Pending::default();
    p.add(easy, 90_000);
    // While 90 seconds are being written, the player solves two more.
    p.add(easy, 70_000);
    p.add(easy, 70_000);
    assert!(p.done(easy, 90_000));
    assert_eq!(p.of(easy).collect::<Vec<_>>(), [70_000, 70_000]);
    assert!(p.done(easy, 70_000));
    assert_eq!(p.of(easy).collect::<Vec<_>>(), [70_000]);
    assert!(!p.done(easy, 90_000));
    assert!(p.done(easy, 70_000));
    assert!(p.is_empty());
    assert!(!p.done(easy, 70_000));
}
