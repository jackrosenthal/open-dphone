//! The leaderboard: the best scores and who set them, as lines of text.
//! Nothing here talks to the phone.  The app keeps one such list in a
//! shared key on the PBX, which every phone reads and adds to, and another
//! in its storage on the phone: the scores it has yet to add.
//!
//! A user is on the leaderboard once, with their best score.

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
    pub fn new(score: u32, id: &str, name: &str) -> Row {
        Row {
            score,
            id: field(id),
            name: field(name).chars().take(NAME_CHARS).collect(),
        }
    }
}

/// The rows of a value: a line for each, of the score, the user's id, and
/// the user's name, with a tab between them.  A line that is no row is
/// skipped.
pub fn parse(value: &[u8]) -> Vec<Row> {
    let rows: Vec<Row> = String::from_utf8_lossy(value)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let score = fields.next()?.trim().parse().ok()?;
            let id = fields.next()?;
            Some(Row::new(score, id, fields.next().unwrap_or("")))
        })
        .filter(|row| row.score > 0)
        .collect();
    // In order, and each user once, whatever was written.
    merge(&[], &rows)
}

/// The value of the rows.
pub fn format(rows: &[Row]) -> Vec<u8> {
    let mut out = String::new();
    for row in rows {
        out.push_str(&format!("{}\t{}\t{}\n", row.score, row.id, row.name));
    }
    out.into_bytes()
}

/// `rows` with `more` added: the best score of each user, best first, and
/// at most [`SIZE`] of them.  Of two equal scores the one that was there
/// first stays above.
pub fn merge(rows: &[Row], more: &[Row]) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::with_capacity(SIZE + 1);
    for row in rows.iter().chain(more) {
        if row.score == 0 {
            continue;
        }
        if let Some(at) = out.iter().position(|r| r.id == row.id) {
            if out[at].score >= row.score {
                continue;
            }
            out.remove(at);
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
                Row::new(row.score, id, name)
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

    fn row(score: u32, id: &str, name: &str) -> Row {
        Row::new(score, id, name)
    }

    #[test]
    fn a_value_is_a_line_for_each_row() {
        let rows = vec![row(12, "101", "Alice Example"), row(7, "102", "Bob")];
        let value = format(&rows);
        assert_eq!(value, b"12\t101\tAlice Example\n7\t102\tBob\n");
        assert_eq!(parse(&value), rows);
        assert_eq!(parse(b""), []);
        assert!(format(&[]).is_empty());
    }

    #[test]
    fn what_is_no_row_is_skipped() {
        let value = b"junk\n9\t101\tAlice\n\n-3\t102\tBob\n0\t103\tCarol\n4\t104\n\xFF\n8";
        assert_eq!(parse(value), [row(9, "101", "Alice"), row(4, "104", "")]);
        // Out of order, a user twice, and more rows than the leaderboard
        // has: it is put right.
        let value = b"1\ta\tA\n5\tb\tB\n3\ta\tA\n2\tc\tC\n9\td\tD\n4\te\tE\n6\tf\tF\n";
        let scores: Vec<(u32, String)> =
            parse(value).into_iter().map(|r| (r.score, r.id)).collect();
        assert_eq!(
            scores,
            [(9, "d"), (6, "f"), (5, "b"), (4, "e"), (3, "a")].map(|(s, i)| (s, i.to_string()))
        );
    }

    #[test]
    fn a_name_cannot_break_a_line() {
        let r = row(3, "10\t1", " Al\tice\nSmith ");
        assert_eq!((r.id.as_str(), r.name.as_str()), ("10 1", "Al ice Smith"));
        assert_eq!(parse(&format(std::slice::from_ref(&r))), [r]);
        let long = row(3, "101", &"é".repeat(100));
        assert_eq!(long.name.chars().count(), 40);
        // Five rows of the longest names are far below the 4096 bytes a
        // value may have.
        let rows: Vec<Row> = (1..=5)
            .map(|i| row(u32::MAX - i, &"9".repeat(64 + i as usize), &"é".repeat(100)))
            .collect();
        assert!(format(&merge(&[], &rows)).len() < 1024);
    }

    #[test]
    fn the_best_of_each_user_best_first() {
        let board = vec![row(9, "101", "Alice"), row(5, "102", "Bob")];
        // A better score of a user replaces that user's row.
        assert_eq!(
            merge(&board, &[row(10, "102", "Bob")]),
            [row(10, "102", "Bob"), row(9, "101", "Alice")]
        );
        // A worse or equal one changes nothing.
        assert_eq!(merge(&board, &[row(5, "102", "Robert")]), board);
        assert_eq!(merge(&board, &[row(4, "102", "Bob")]), board);
        // A new user goes below an equal score that was there first.
        assert_eq!(
            merge(&board, &[row(9, "103", "Carol")]),
            [
                row(9, "101", "Alice"),
                row(9, "103", "Carol"),
                row(5, "102", "Bob")
            ]
        );
        // A score of zero is no score.
        assert_eq!(merge(&board, &[row(0, "104", "Dan")]), board);
    }

    #[test]
    fn at_most_five_rows() {
        let full: Vec<Row> = (0..5).map(|i| row(10 - i, &format!("u{i}"), "N")).collect();
        assert_eq!(merge(&[], &full), full);
        // Too low for a full leaderboard.
        assert_eq!(merge(&full, &[row(6, "new", "New")]), full);
        assert_eq!(merge(&full, &[row(5, "new", "New")]), full);
        // High enough: the last row goes.
        let with = merge(&full, &[row(8, "new", "New")]);
        assert_eq!(with.len(), 5);
        assert_eq!(with[3], row(8, "new", "New"));
        assert_eq!(with[4], full[3]);
    }

    #[test]
    fn what_is_still_to_be_written() {
        let board = vec![row(9, "101", "Alice"), row(5, "102", "Bob")];
        let waiting = vec![
            row(7, "102", "Bob"),
            row(8, "101", "Alice"),
            row(1, "103", "Carol"),
        ];
        assert_eq!(
            unsettled(&board, &waiting),
            [row(7, "102", "Bob"), row(1, "103", "Carol")]
        );
        // Once they are on the leaderboard, nothing is.
        let board = merge(&board, &waiting);
        assert!(unsettled(&board, &waiting).is_empty());
        // And nothing that is too low for a full one.
        let full: Vec<Row> = (0..5).map(|i| row(10 - i, &format!("u{i}"), "N")).collect();
        assert!(unsettled(&full, &[row(6, "new", "New")]).is_empty());
    }

    #[test]
    fn scores_of_an_unknown_user_become_the_users() {
        let waiting = merge(&[], &[row(4, "", ""), row(6, "101", "Alice")]);
        assert_eq!(waiting, [row(6, "101", "Alice"), row(4, "", "")]);
        // The same unknown user: the better score.
        assert_eq!(merge(&waiting, &[row(5, "", "")])[1], row(5, "", ""));
        assert_eq!(
            claim(&waiting, "102", "Bob"),
            [row(6, "101", "Alice"), row(4, "102", "Bob")]
        );
        assert_eq!(claim(&waiting, "101", "Alice"), [row(6, "101", "Alice")]);
    }
}
