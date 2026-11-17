//! The tables of the best times, and the times that wait to be written.

use lightup_game::generate::{Difficulty, SIZES};
use lightup_game::scores::{Kind, NAME_CHARS, Pending, ROWS, Table, Win, clock};

/// Nov 13, 2026, 2:15 PM UTC.
const T: u64 = 1_794_579_300;

fn win(ms: u64, when: u64) -> Win {
    Win { ms, when }
}

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
    assert!(t.add(win(90_000, T), "101", "Alice"));
    assert!(t.add(win(60_000, T + 60), "102", "Bob"));
    assert!(t.add(win(120_000, T + 120), "103", "Carol"));
    // As good as Bob's, and later.
    assert!(t.add(win(60_000, T + 180), "104", "Dan"));
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
    t.add(win(90_000, T), "101", "Alice");
    t.add(win(60_000, T + 60), "102", "Bob");
    // Worse than her other time, and better: each is a row beside it,
    // with the name the PBX had when it was written.
    assert!(t.add(win(95_000, T + 600), "101", "Alice"));
    assert!(t.add(win(50_000, T + 900), "101", "Alice Smith"));
    assert_eq!(
        names(&t),
        [("Alice Smith", 50_000), ("Bob", 60_000), ("Alice", 90_000), ("Alice", 95_000)]
    );
    // Each with when it was solved.
    let when: Vec<u64> = t.rows().iter().map(|r| r.when).collect();
    assert_eq!(when, [T + 900, T + 60, T, T + 600]);
    // Her best row.
    assert_eq!(t.place("101"), Some(0));
    assert_eq!(t.place("102"), Some(1));
    // A win of hers that the table has is that win, written before: the
    // table is as it was.
    let before = t.clone();
    assert!(!t.add(win(90_000, T), "101", "Alice"));
    assert!(!t.add(win(50_000, T + 900), "101", "Alice Smith"));
    assert_eq!(t, before);
    // Another player's, or another millisecond, is another win.
    assert!(t.add(win(90_000, T), "102", "Bob"));
    assert!(t.add(win(90_001, T), "101", "Alice"));
    assert_eq!(t.rows().len(), 6);
    // Two players of one name are two players.
    assert!(t.add(win(70_000, T + 1200), "105", "Bob"));
    assert_eq!(t.rows().len(), 7);
}

#[test]
fn two_wins_of_the_same_time_are_two_rows() {
    let easy = Kind::new(7, Difficulty::Easy);
    // Alice solves two puzzles in the very same time, ten minutes apart.
    let (one, two) = (win(83_000, T), win(83_000, T + 600));
    let mut t = Table::default();
    assert!(t.add(one, "101", "Alice"));
    assert!(t.add(two, "101", "Alice"));
    assert_eq!(names(&t), [("Alice", 83_000), ("Alice", 83_000)]);
    assert_eq!(t.rows()[0].win(), one);
    assert_eq!(t.rows()[1].win(), two);
    assert_eq!(Table::parse(&t.to_bytes()), t);
    // Both wait to be written, and each is written.
    let mut p = Pending::default();
    assert!(p.add(easy, one));
    assert!(p.add(easy, two));
    assert_eq!(Pending::parse(&p.to_text()), p);
    let mut written = Table::default();
    for w in p.of(easy) {
        assert!(written.add(w, "101", "Alice"));
    }
    assert_eq!(written, t);
    // Written again (the answers did not come): each is there once.
    for w in p.of(easy) {
        assert!(!written.add(w, "101", "Alice"));
    }
    assert_eq!(written, t);
    // Each waits until it is written.
    assert!(p.done(easy, one));
    assert_eq!(p.of(easy).collect::<Vec<_>>(), [two]);
    assert!(!p.done(easy, one));
    assert!(p.done(easy, two));
    assert!(p.is_empty());
}

#[test]
fn a_table_keeps_the_ten_best() {
    let mut t = Table::default();
    for n in 0..ROWS as u64 {
        let id = format!("{}", 100 + n);
        assert!(t.add(win(10_000 * (n + 1), T + n), &id, &format!("P{n}")));
    }
    assert_eq!(t.rows().len(), 10);
    // Not among the best, also when as good as the last: nothing changes.
    let before = t.clone();
    assert!(!t.add(win(100_000, T + 100), "200", "Late"));
    assert!(!t.add(win(500_000, T + 101), "200", "Slow"));
    assert_eq!(t, before);
    // Among them: the last one leaves.
    assert!(t.add(win(15_000, T + 102), "200", "Quick"));
    assert_eq!(t.rows().len(), 10);
    assert_eq!(t.rows()[1].name, "Quick");
    assert_eq!(t.place("109"), None);
    // A player's better time is a row of its own: the last row leaves,
    // and the player's other row stays.
    assert!(t.add(win(12_000, T + 103), "103", "P3"));
    assert_eq!(t.rows().len(), 10);
    assert_eq!(t.rows()[1].name, "P3");
    assert_eq!(t.rows()[5].name, "P3");
    assert_eq!(t.rows()[9].name, "P7");
    assert_eq!(t.place("103"), Some(1));
    assert_eq!(t.place("108"), None);
    // A table can be one player's.
    let mut t = Table::default();
    for n in 0..ROWS as u64 + 2 {
        assert!(t.add(win(60_000 - n, T + n), "101", "Alice"));
    }
    assert_eq!(t.rows().len(), 10);
    assert_eq!(t.rows()[0].ms, 60_000 - ROWS as u64 - 1);
    assert_eq!(t.rows()[9].ms, 60_000 - 2);
}

#[test]
fn a_table_is_text_and_reads_back_as_it_was() {
    let mut t = Table::default();
    t.add(win(90_000, T), "101", "Alice");
    t.add(win(61_234, T + 60), "102", "Bob O'Neil, Jr.");
    t.add(win(75_500, T + 3600), "101", "Alice");
    let bytes = t.to_bytes();
    assert_eq!(Table::parse(&bytes), t);
    assert_eq!(Table::default().to_bytes(), b"");
    assert_eq!(Table::parse(b""), Table::default());
}

#[test]
fn a_name_cannot_break_a_table() {
    let mut t = Table::default();
    t.add(win(1_000, T), "101", "  Al\tice\nSmith ");
    t.add(win(2_000, T), "1\t02", &"x".repeat(100));
    assert_eq!(t.rows()[0].name, "Al ice Smith");
    assert_eq!(t.rows()[1].id, "1 02");
    assert_eq!(t.rows()[1].name.chars().count(), NAME_CHARS);
    assert_eq!(Table::parse(&t.to_bytes()), t);
    // Ten rows of the longest names, in letters of four bytes, with long
    // ids and the longest numbers, are within a value of the store (4096
    // bytes).
    let mut t = Table::default();
    for n in 0..ROWS as u64 {
        let id = format!("{n}{}", "i".repeat(63));
        t.add(win(u64::MAX - n, u64::MAX), &id, &"\u{1F4A1}".repeat(100));
    }
    assert_eq!(t.rows().len(), ROWS);
    assert!(t.to_bytes().len() <= 4096, "{}", t.to_bytes().len());
}

#[test]
fn what_is_not_a_table_is_read_as_what_it_holds_of_one() {
    // Not rows (no number, too few fields, no moment, or no number
    // for it), a row twice, out of order, and too many.
    let t = Table::parse(
        b"nonsense\n\n70000\t103\tCarol\t1794579300\nfast\t104\tDan\t1794579300\n\
          60000\t101\tAlice\t1794579300\n65000\t101\tAlice\t1794579300\n80000\t105\n\
          85000\t105\tEd\n86000\t107\tFay\tsoon\n\xff\xfe\n\
          90000\t106\tEve\t1794579300\twith more\n60000\t101\tAlice\t1794579300\n\
          60000\t101\tAlice\t1794580000\n",
    );
    // The same time of Alice's at another moment is another row.
    assert_eq!(
        names(&t),
        [
            ("Alice", 60_000),
            ("Alice", 60_000),
            ("Alice", 65_000),
            ("Carol", 70_000),
            ("Eve", 90_000)
        ]
    );
    let mut many = String::new();
    for n in 0..30 {
        many.push_str(&format!("{}\t{n}\tP{n}\t{}\n", 1000 + n, T + n));
    }
    assert_eq!(Table::parse(many.as_bytes()).rows().len(), ROWS);
}

#[test]
fn every_time_waits() {
    let easy = Kind::new(7, Difficulty::Easy);
    let hard = Kind::new(14, Difficulty::Hard);
    let mut p = Pending::default();
    assert!(p.is_empty());
    let wins = [
        (easy, win(90_000, T)),
        (hard, win(600_000, T + 60)),
        (easy, win(95_000, T + 120)),
        (easy, win(80_000, T + 180)),
        (easy, win(80_000, T + 240)),
    ];
    for (kind, w) in wins {
        assert!(p.add(kind, w));
    }
    assert_eq!(p.times(), wins);
    let ms: Vec<u64> = p.of(easy).map(|w| w.ms).collect();
    assert_eq!(ms, [90_000, 95_000, 80_000, 80_000]);
    assert_eq!(p.of(Kind::new(10, Difficulty::Easy)).count(), 0);

    assert_eq!(Pending::parse(&p.to_text()), p);
    assert_eq!(
        Pending::parse(
            "7x7-easy 90000 1794579300\nnonsense\n14x14-hard 600000 1794579360\n\
             9x9-easy 5 1794579300\n7x7-easy 95000 1794579420\n7x7-hard soon 1794579300\n\
             7x7-easy 70000\n7x7-easy 70000 later\n7x7-easy 80000 1794579480\n\
             7x7-easy 80000 1794579540\n"
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
    p.add(hard, win(600_000, T));
    for n in 0..ROWS as u64 {
        assert!(p.add(easy, win(60_000 + 1_000 * (n % 5), T + 60 * n)));
    }
    let full = p.clone();
    // Worse than each of them, or as good as the worst: it could not
    // make the table once they are written.
    assert!(!p.add(easy, win(70_000, T + 3600)));
    assert!(!p.add(easy, win(64_000, T + 3660)));
    assert_eq!(p, full);
    // Better: the worst leaves, of two equals the one that came later.
    assert!(p.add(easy, win(50_000, T + 3720)));
    let easy_times: Vec<u64> = p.of(easy).map(|w| w.ms).collect();
    assert_eq!(
        easy_times,
        [60_000, 61_000, 62_000, 63_000, 64_000, 60_000, 61_000, 62_000, 63_000, 50_000]
    );
    assert_eq!(p.of(easy).nth(4), Some(win(64_000, T + 240)));
    // The other kinds have room of their own.
    assert!(p.add(hard, win(900_000, T + 3780)));
    assert_eq!(p.of(hard).count(), 2);
    // So does a file with too many.
    let mut many = String::new();
    for n in 0..30 {
        many.push_str(&format!("7x7-easy {} {}\n", 90_000 - n, T + n));
    }
    let read = Pending::parse(&many);
    assert_eq!(read.of(easy).count(), ROWS);
    assert_eq!(read.of(easy).map(|w| w.ms).max(), Some(90_000 - 20));
}

#[test]
fn a_written_time_waits_no_longer_and_the_others_still_do() {
    let easy = Kind::new(7, Difficulty::Easy);
    let mut p = Pending::default();
    let first = win(90_000, T);
    p.add(easy, first);
    // While 90 seconds are being written, the player solves two more,
    // in the same time.
    let (second, third) = (win(70_000, T + 300), win(70_000, T + 600));
    p.add(easy, second);
    p.add(easy, third);
    assert!(p.done(easy, first));
    assert_eq!(p.of(easy).collect::<Vec<_>>(), [second, third]);
    assert!(p.done(easy, third));
    assert_eq!(p.of(easy).collect::<Vec<_>>(), [second]);
    assert!(!p.done(easy, first));
    assert!(!p.done(easy, third));
    assert!(p.done(easy, second));
    assert!(p.is_empty());
    assert!(!p.done(easy, second));
}
