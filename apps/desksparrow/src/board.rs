//! The leaderboard: the best scores and who set them, as lines of text.
//! Nothing here talks to the phone.  The app keeps one such list in a
//! shared key on the PBX, which every phone reads and adds to, and another
//! in its storage on the phone: the scores it has yet to add.
//!
//! Every score is a row of its own: a user can be on the leaderboard
//! several times.  A row has a tag that tells it from another of the same
//! score and user, so that a row written twice is still one row.

/// How many scores the leaderboard keeps: as many as the smallest screen
/// shows under a title.
pub const SIZE: usize = 5;

/// The most characters of a name that are kept.
const NAME_CHARS: usize = 40;

/// A score and who set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub score: u32,
    /// The user (`PbxUser::id`).  Empty for a score set before the phone
    /// knew its user.
    pub id: String,
    /// The user's name when they set it.
    pub name: String,
    /// Tells this row from the others: random, chosen when the score was
    /// set.  Empty for a score set before rows had one.
    pub tag: String,
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
    pub fn new(score: u32, id: &str, name: &str, tag: &str) -> Row {
        Row {
            score,
            id: field(id),
            name: field(name).chars().take(NAME_CHARS).collect(),
            tag: field(tag),
        }
    }
}

/// The rows of a value: a line for each, of the score, the user's id, the
/// user's name, and the tag, with a tab between them.  A line without a
/// tag is a row with an empty one.  A line that is no row is skipped.
pub fn parse(value: &[u8]) -> Vec<Row> {
    let rows: Vec<Row> = String::from_utf8_lossy(value)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let score = fields.next()?.trim().parse().ok()?;
            let id = fields.next()?;
            let name = fields.next().unwrap_or("");
            Some(Row::new(score, id, name, fields.next().unwrap_or("")))
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
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            row.score, row.id, row.name, row.tag
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
                Row::new(row.score, id, name, &row.tag)
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

    fn row(score: u32, id: &str, name: &str, tag: &str) -> Row {
        Row::new(score, id, name, tag)
    }

    #[test]
    fn a_value_is_a_line_for_each_row() {
        let rows = vec![
            row(12, "101", "Alice Example", "a1"),
            row(7, "102", "Bob", "b1"),
        ];
        let value = format(&rows);
        assert_eq!(value, b"12\t101\tAlice Example\ta1\n7\t102\tBob\tb1\n");
        assert_eq!(parse(&value), rows);
        assert_eq!(parse(b""), []);
        assert!(format(&[]).is_empty());
    }

    #[test]
    fn what_is_no_row_is_skipped() {
        let value = b"junk\n9\t101\tAlice\tt\n\n-3\t102\tBob\tt\n0\t103\tCarol\tt\n4\t104\n\xFF\n8";
        assert_eq!(
            parse(value),
            [row(9, "101", "Alice", "t"), row(4, "104", "", "")]
        );
        // Out of order, a user twice, and more rows than the leaderboard
        // has: it is put right.
        let value =
            b"1\ta\tA\tt1\n5\tb\tB\tt2\n3\ta\tA\tt3\n2\tc\tC\tt4\n9\td\tD\tt5\n6\ta\tA\tt6\n";
        let scores: Vec<(u32, String)> =
            parse(value).into_iter().map(|r| (r.score, r.id)).collect();
        assert_eq!(
            scores,
            [(9, "d"), (6, "a"), (5, "b"), (3, "a"), (2, "c")].map(|(s, i)| (s, i.to_string()))
        );
        // A row written twice is there once.
        assert_eq!(
            parse(b"5\tb\tB\tt2\n5\tb\tB\tt2\n"),
            [row(5, "b", "B", "t2")]
        );
    }

    #[test]
    fn a_line_without_a_tag_is_a_row() {
        // As the leaderboard was written before rows had tags.
        let value = b"12\t101\tAlice Example\n7\t102\tBob\n";
        let rows = parse(value);
        assert_eq!(
            rows,
            [
                row(12, "101", "Alice Example", ""),
                row(7, "102", "Bob", "")
            ]
        );
        assert_eq!(format(&rows), b"12\t101\tAlice Example\t\n7\t102\tBob\t\n");
        assert_eq!(parse(&format(&rows)), rows);
        // And the same user's later scores are rows besides it.
        assert_eq!(
            merge(&rows, &[row(12, "101", "Alice Example", "t")]),
            [
                row(12, "101", "Alice Example", ""),
                row(12, "101", "Alice Example", "t"),
                row(7, "102", "Bob", "")
            ]
        );
    }

    #[test]
    fn a_name_cannot_break_a_line() {
        let r = row(3, "10\t1", " Al\tice\nSmith ", "f0\t0d");
        assert_eq!(
            (r.id.as_str(), r.name.as_str(), r.tag.as_str()),
            ("10 1", "Al ice Smith", "f0 0d")
        );
        assert_eq!(parse(&format(std::slice::from_ref(&r))), [r]);
        let long = row(3, "101", &"é".repeat(100), "t");
        assert_eq!(long.name.chars().count(), 40);
        // Five rows of the longest names are far below the 4096 bytes a
        // value may have.
        let rows: Vec<Row> = (1..=5)
            .map(|i| {
                row(
                    u32::MAX - i,
                    &"9".repeat(64 + i as usize),
                    &"é".repeat(100),
                    &format!("{:016x}", u64::MAX - u64::from(i)),
                )
            })
            .collect();
        assert!(format(&merge(&[], &rows)).len() < 1024);
    }

    #[test]
    fn every_score_best_first() {
        let board = vec![row(9, "101", "Alice", "a1"), row(5, "102", "Bob", "b1")];
        // A better score of a user is a row of its own, and so is a worse
        // one or an equal one.
        assert_eq!(
            merge(&board, &[row(10, "102", "Bob", "b2")]),
            [
                row(10, "102", "Bob", "b2"),
                row(9, "101", "Alice", "a1"),
                row(5, "102", "Bob", "b1")
            ]
        );
        assert_eq!(
            merge(&board, &[row(4, "102", "Bob", "b2")]),
            [
                row(9, "101", "Alice", "a1"),
                row(5, "102", "Bob", "b1"),
                row(4, "102", "Bob", "b2")
            ]
        );
        // A new score goes below an equal score that was there first.
        assert_eq!(
            merge(&board, &[row(5, "102", "Bob", "b2")]),
            [
                row(9, "101", "Alice", "a1"),
                row(5, "102", "Bob", "b1"),
                row(5, "102", "Bob", "b2")
            ]
        );
        assert_eq!(
            merge(&board, &[row(9, "103", "Carol", "c1")]),
            [
                row(9, "101", "Alice", "a1"),
                row(9, "103", "Carol", "c1"),
                row(5, "102", "Bob", "b1")
            ]
        );
        // A row that is there already changes nothing.
        assert_eq!(merge(&board, &board), board);
        assert_eq!(merge(&board, &[row(5, "102", "Bob", "b1")]), board);
        // A score of zero is no score.
        assert_eq!(merge(&board, &[row(0, "104", "Dan", "d1")]), board);
    }

    #[test]
    fn at_most_five_rows() {
        let full: Vec<Row> = (0..5)
            .map(|i| row(10 - i, &format!("u{i}"), "N", &format!("t{i}")))
            .collect();
        assert_eq!(merge(&[], &full), full);
        // Too low for a full leaderboard.
        assert_eq!(merge(&full, &[row(6, "new", "New", "n")]), full);
        assert_eq!(merge(&full, &[row(5, "new", "New", "n")]), full);
        // High enough: the last row goes.
        let with = merge(&full, &[row(8, "new", "New", "n")]);
        assert_eq!(with.len(), 5);
        assert_eq!(with[3], row(8, "new", "New", "n"));
        assert_eq!(with[4], full[3]);
        // One user can have all five rows.
        let one: Vec<Row> = (1..=6)
            .map(|i| row(i, "101", "Alice", &format!("a{i}")))
            .collect();
        let scores: Vec<u32> = merge(&[], &one).iter().map(|r| r.score).collect();
        assert_eq!(scores, [6, 5, 4, 3, 2]);
    }

    #[test]
    fn what_is_still_to_be_written() {
        let board = vec![row(9, "101", "Alice", "a1"), row(5, "102", "Bob", "b1")];
        let waiting = vec![
            row(7, "102", "Bob", "b2"),
            row(8, "101", "Alice", "a2"),
            row(5, "102", "Bob", "b1"),
            row(1, "103", "Carol", "c1"),
        ];
        // All but the one that is there already.
        assert_eq!(
            unsettled(&board, &waiting),
            [
                row(7, "102", "Bob", "b2"),
                row(8, "101", "Alice", "a2"),
                row(1, "103", "Carol", "c1")
            ]
        );
        // Once they are on the leaderboard, nothing is.
        let board = merge(&board, &waiting);
        assert_eq!(board.len(), 5);
        assert!(unsettled(&board, &waiting).is_empty());
        // And nothing that is too low for a full one.
        let full: Vec<Row> = (0..5)
            .map(|i| row(10 - i, &format!("u{i}"), "N", &format!("t{i}")))
            .collect();
        assert!(unsettled(&full, &[row(6, "new", "New", "n")]).is_empty());
    }

    #[test]
    fn scores_of_an_unknown_user_become_the_users() {
        let waiting = merge(
            &[],
            &[
                row(4, "", "", "x1"),
                row(6, "101", "Alice", "a1"),
                row(5, "", "", "x2"),
            ],
        );
        assert_eq!(
            waiting,
            [
                row(6, "101", "Alice", "a1"),
                row(5, "", "", "x2"),
                row(4, "", "", "x1")
            ]
        );
        assert_eq!(
            claim(&waiting, "102", "Bob"),
            [
                row(6, "101", "Alice", "a1"),
                row(5, "102", "Bob", "x2"),
                row(4, "102", "Bob", "x1")
            ]
        );
        assert_eq!(
            claim(&waiting, "101", "Alice"),
            [
                row(6, "101", "Alice", "a1"),
                row(5, "101", "Alice", "x2"),
                row(4, "101", "Alice", "x1")
            ]
        );
    }
}
