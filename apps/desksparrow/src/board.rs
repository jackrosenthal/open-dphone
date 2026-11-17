//! The leaderboard: the best scores, who set them, and when, as lines of
//! text.  Nothing here talks to the phone.  The app keeps one such list in
//! a shared key on the PBX, which every phone reads and adds to, and
//! another in its storage on the phone: the scores it has yet to add.
//!
//! Every score is a row of its own: a user can be on the leaderboard
//! several times.  A row has the date and time the score was set, which
//! tells it from another of the same score and user, so that a row written
//! twice is still one row.

/// How many scores the leaderboard keeps: as many as the smallest screen
/// shows under a title.
pub const SIZE: usize = 5;

/// The most characters of a name that are kept.
const NAME_CHARS: usize = 40;

/// A score, who set it, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub score: u32,
    /// The user (`PbxUser::id`).  Empty for a score set before the phone
    /// knew its user.
    pub id: String,
    /// The user's name when they set it.
    pub name: String,
    /// When the score was set, in seconds since the Unix epoch.  `None`
    /// for a score set before rows had one.
    pub time: Option<u64>,
}

/// A field of a line: no tabs or line ends, which separate fields and
/// lines.
fn field(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

impl Row {
    pub fn new(score: u32, id: &str, name: &str, time: Option<u64>) -> Row {
        Row {
            score,
            id: field(id),
            name: field(name).chars().take(NAME_CHARS).collect(),
            time,
        }
    }
}

/// The rows of a value: a line for each, of the score, the user's id, the
/// user's name, and the time the score was set in seconds since the Unix
/// epoch, with a tab between them.  A line without a time is a row
/// without one.  A line that is no row is skipped.
pub fn parse(value: &[u8]) -> Vec<Row> {
    let rows: Vec<Row> = String::from_utf8_lossy(value)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let score = fields.next()?.trim().parse().ok()?;
            let id = fields.next()?;
            let name = fields.next().unwrap_or("");
            let time = fields.next().and_then(|t| t.trim().parse().ok());
            Some(Row::new(score, id, name, time))
        })
        .filter(|row| row.score > 0)
        .collect();
    // In order, and each row once, whatever was written.
    merge(&[], &rows)
}

/// The value of the rows.
pub fn format(rows: &[Row]) -> Vec<u8> {
    let mut out = String::new();
    for row in rows {
        let time = row.time.map(|t| t.to_string()).unwrap_or_default();
        out.push_str(&format!(
            "{}\t{}\t{}\t{time}\n",
            row.score, row.id, row.name
        ));
    }
    out.into_bytes()
}

/// `rows` with `more` added: every row once, best first, and at most
/// [`SIZE`] of them.  Of two equal scores the one that was there first
/// stays above.
pub fn merge(rows: &[Row], more: &[Row]) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::with_capacity(SIZE + 1);
    for row in rows.iter().chain(more) {
        if row.score == 0 || out.contains(row) {
            continue;
        }
        let at = out
            .iter()
            .position(|r| r.score < row.score)
            .unwrap_or(out.len());
        out.insert(at, row.clone());
    }
    out.truncate(SIZE);
    out
}

/// The rows of `waiting` that would change `board` if they were added to
/// it: the ones still to be written there.
pub fn unsettled(board: &[Row], waiting: &[Row]) -> Vec<Row> {
    waiting
        .iter()
        .filter(|row| merge(board, std::slice::from_ref(row)) != board)
        .cloned()
        .collect()
}

/// `waiting` with the rows of an unknown user made the rows of the user
/// `id` named `name`.
pub fn claim(waiting: &[Row], id: &str, name: &str) -> Vec<Row> {
    let claimed: Vec<Row> = waiting
        .iter()
        .map(|row| {
            if row.id.is_empty() {
                Row::new(row.score, id, name, row.time)
            } else {
                row.clone()
            }
        })
        .collect();
    merge(&[], &claimed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row set at `time`.
    fn row(score: u32, id: &str, name: &str, time: u64) -> Row {
        Row::new(score, id, name, Some(time))
    }

    /// A row set before rows had a time.
    fn untimed(score: u32, id: &str, name: &str) -> Row {
        Row::new(score, id, name, None)
    }

    #[test]
    fn a_value_is_a_line_for_each_row() {
        let rows = vec![
            row(12, "101", "Alice Example", 1_794_865_500),
            row(7, "102", "Bob", 1_794_865_920),
        ];
        let value = format(&rows);
        assert_eq!(
            value,
            b"12\t101\tAlice Example\t1794865500\n7\t102\tBob\t1794865920\n"
        );
        assert_eq!(parse(&value), rows);
        assert_eq!(parse(b""), []);
        assert!(format(&[]).is_empty());
    }

    #[test]
    fn what_is_no_row_is_skipped() {
        let value = b"junk\n9\t101\tAlice\t5\n\n-3\t102\tBob\t5\n0\t103\tCarol\t5\n4\t104\n\xFF\n8";
        assert_eq!(
            parse(value),
            [row(9, "101", "Alice", 5), untimed(4, "104", "")]
        );
        // Out of order, a user twice, and more rows than the leaderboard
        // has: it is put right.
        let value = b"1\ta\tA\t1\n5\tb\tB\t2\n3\ta\tA\t3\n2\tc\tC\t4\n9\td\tD\t5\n6\ta\tA\t6\n";
        let scores: Vec<(u32, String)> =
            parse(value).into_iter().map(|r| (r.score, r.id)).collect();
        assert_eq!(
            scores,
            [(9, "d"), (6, "a"), (5, "b"), (3, "a"), (2, "c")].map(|(s, i)| (s, i.to_string()))
        );
        // A row written twice is there once.
        assert_eq!(parse(b"5\tb\tB\t2\n5\tb\tB\t2\n"), [row(5, "b", "B", 2)]);
    }

    #[test]
    fn a_line_without_a_time_is_a_row() {
        // As the leaderboard was written before rows had times.
        let value = b"12\t101\tAlice Example\n7\t102\tBob\n";
        let rows = parse(value);
        assert_eq!(
            rows,
            [
                untimed(12, "101", "Alice Example"),
                untimed(7, "102", "Bob")
            ]
        );
        assert_eq!(format(&rows), b"12\t101\tAlice Example\t\n7\t102\tBob\t\n");
        assert_eq!(parse(&format(&rows)), rows);
        // A fourth field that is no time is none.
        assert_eq!(
            parse(b"7\t102\tBob\t0123456789abcdef\n"),
            [untimed(7, "102", "Bob")]
        );
        // And the same user's later scores are rows besides it.
        assert_eq!(
            merge(&rows, &[row(12, "101", "Alice Example", 9)]),
            [
                untimed(12, "101", "Alice Example"),
                row(12, "101", "Alice Example", 9),
                untimed(7, "102", "Bob")
            ]
        );
    }

    #[test]
    fn a_name_cannot_break_a_line() {
        let r = row(3, "10\t1", " Al\tice\nSmith ", 7);
        assert_eq!((r.id.as_str(), r.name.as_str()), ("10 1", "Al ice Smith"));
        assert_eq!(parse(&format(std::slice::from_ref(&r))), [r]);
        let long = row(3, "101", &"é".repeat(100), 7);
        assert_eq!(long.name.chars().count(), 40);
        // Five rows of the longest names and times are far below the 4096
        // bytes a value may have.
        let rows: Vec<Row> = (1..=5)
            .map(|i| {
                row(
                    u32::MAX - i,
                    &"9".repeat(64 + i as usize),
                    &"é".repeat(100),
                    u64::MAX - u64::from(i),
                )
            })
            .collect();
        assert!(format(&merge(&[], &rows)).len() < 1024);
    }

    #[test]
    fn every_score_best_first() {
        let board = vec![row(9, "101", "Alice", 1), row(5, "102", "Bob", 2)];
        // A better score of a user is a row of its own, and so is a worse
        // one or an equal one.
        assert_eq!(
            merge(&board, &[row(10, "102", "Bob", 3)]),
            [
                row(10, "102", "Bob", 3),
                row(9, "101", "Alice", 1),
                row(5, "102", "Bob", 2)
            ]
        );
        assert_eq!(
            merge(&board, &[row(4, "102", "Bob", 3)]),
            [
                row(9, "101", "Alice", 1),
                row(5, "102", "Bob", 2),
                row(4, "102", "Bob", 3)
            ]
        );
        // A new score goes below an equal score that was there first.
        assert_eq!(
            merge(&board, &[row(5, "102", "Bob", 3)]),
            [
                row(9, "101", "Alice", 1),
                row(5, "102", "Bob", 2),
                row(5, "102", "Bob", 3)
            ]
        );
        assert_eq!(
            merge(&board, &[row(9, "103", "Carol", 3)]),
            [
                row(9, "101", "Alice", 1),
                row(9, "103", "Carol", 3),
                row(5, "102", "Bob", 2)
            ]
        );
        // A row that is there already changes nothing: the same score of
        // the same user set at the same time.
        assert_eq!(merge(&board, &board), board);
        assert_eq!(merge(&board, &[row(5, "102", "Bob", 2)]), board);
        // A score of zero is no score.
        assert_eq!(merge(&board, &[row(0, "104", "Dan", 3)]), board);
    }

    #[test]
    fn at_most_five_rows() {
        let full: Vec<Row> = (0..5)
            .map(|i| row(10 - i, &format!("u{i}"), "N", u64::from(i)))
            .collect();
        assert_eq!(merge(&[], &full), full);
        // Too low for a full leaderboard.
        assert_eq!(merge(&full, &[row(6, "new", "New", 9)]), full);
        assert_eq!(merge(&full, &[row(5, "new", "New", 9)]), full);
        // High enough: the last row goes.
        let with = merge(&full, &[row(8, "new", "New", 9)]);
        assert_eq!(with.len(), 5);
        assert_eq!(with[3], row(8, "new", "New", 9));
        assert_eq!(with[4], full[3]);
        // One user can have all five rows.
        let one: Vec<Row> = (1..=6)
            .map(|i| row(i, "101", "Alice", u64::from(i)))
            .collect();
        let scores: Vec<u32> = merge(&[], &one).iter().map(|r| r.score).collect();
        assert_eq!(scores, [6, 5, 4, 3, 2]);
    }

    #[test]
    fn what_is_still_to_be_written() {
        let board = vec![row(9, "101", "Alice", 1), row(5, "102", "Bob", 2)];
        let waiting = vec![
            row(7, "102", "Bob", 3),
            row(8, "101", "Alice", 4),
            row(5, "102", "Bob", 2),
            row(1, "103", "Carol", 5),
        ];
        // All but the one that is there already.
        assert_eq!(
            unsettled(&board, &waiting),
            [
                row(7, "102", "Bob", 3),
                row(8, "101", "Alice", 4),
                row(1, "103", "Carol", 5)
            ]
        );
        // Once they are on the leaderboard, nothing is.
        let board = merge(&board, &waiting);
        assert_eq!(board.len(), 5);
        assert!(unsettled(&board, &waiting).is_empty());
        // And nothing that is too low for a full one.
        let full: Vec<Row> = (0..5)
            .map(|i| row(10 - i, &format!("u{i}"), "N", u64::from(i)))
            .collect();
        assert!(unsettled(&full, &[row(6, "new", "New", 9)]).is_empty());
    }

    #[test]
    fn scores_of_an_unknown_user_become_the_users() {
        let waiting = merge(
            &[],
            &[
                row(4, "", "", 1),
                row(6, "101", "Alice", 2),
                row(5, "", "", 3),
            ],
        );
        assert_eq!(
            waiting,
            [
                row(6, "101", "Alice", 2),
                row(5, "", "", 3),
                row(4, "", "", 1)
            ]
        );
        assert_eq!(
            claim(&waiting, "102", "Bob"),
            [
                row(6, "101", "Alice", 2),
                row(5, "102", "Bob", 3),
                row(4, "102", "Bob", 1)
            ]
        );
        assert_eq!(
            claim(&waiting, "101", "Alice"),
            [
                row(6, "101", "Alice", 2),
                row(5, "101", "Alice", 3),
                row(4, "101", "Alice", 1)
            ]
        );
    }
}
