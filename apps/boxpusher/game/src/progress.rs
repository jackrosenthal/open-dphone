//! How far a player has come, as it is written down on the phone and on
//! the PBX, and which of the two counts when they differ.

use crate::LEVELS;

/// The most steps a written progress holds.  A level left after more
/// than this starts over the next time; the longest level takes about
/// 120.
pub const STEPS_KEPT: usize = 2000;

/// The value of `name=value` on a line of `text`, or "".
fn field<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
        .unwrap_or("")
}

/// Where a player is: the level being played, how many levels are solved,
/// and the steps taken in the level so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// The level being played, counted from 0.
    pub level: usize,
    /// How many levels are solved: the levels up to this one are open.
    pub solved: usize,
    /// The steps taken in the level, one letter each (see
    /// [`crate::Game::record`]).
    pub moves: String,
}

impl Progress {
    /// Reads a written progress; what it does not say, or says wrongly,
    /// is the beginning.
    pub fn parse(text: &str) -> Progress {
        let number = |name: &str| field(text, name).parse::<usize>().unwrap_or(0);
        let solved = number("solved").min(LEVELS.len());
        Progress {
            level: number("level").min(solved).min(LEVELS.len() - 1),
            solved,
            moves: field(text, "moves").to_string(),
        }
    }

    /// The progress written down: a line each for `level`, `solved`, and
    /// `moves`.
    pub fn text(&self) -> String {
        let moves = if self.moves.len() > STEPS_KEPT {
            ""
        } else {
            &self.moves
        };
        format!(
            "level={}\nsolved={}\nmoves={}\n",
            self.level, self.solved, moves
        )
    }

    /// The progress to go on with, on the phone and on the PBX, when the
    /// phone has `self` and the PBX has `theirs` (its entry's version and
    /// value; `None` if it has nothing).  `known` is what the phone last
    /// knew the PBX to have, and `played` whether the player has pressed
    /// a key since the game came on screen.
    ///
    /// A level solved on either side stays solved.  The level being
    /// played and its steps are the PBX's if the PBX's entry is another
    /// than the phone knew and the phone has nothing new (the phone was
    /// reset, or the user played on another phone), or if both have
    /// something new and the PBX has more levels solved.  Otherwise they
    /// are the phone's: it played while the PBX was out of reach, or
    /// nothing changed on the PBX.  A player who is already playing is
    /// never moved to another level.
    ///
    /// What the phone sent without hearing the answer ([`Known::sent`])
    /// is taken to have arrived if the PBX's entry is another since: it
    /// is nothing new on the phone.
    pub fn reconcile(
        &self,
        known: &Known,
        theirs: Option<(u64, &Progress)>,
        played: bool,
    ) -> Progress {
        let Some((version, theirs)) = theirs else {
            return self.clone();
        };
        let solved = self.solved.max(theirs.solved);
        let new_there = version != known.version;
        let new_here = *self != known.progress && !(new_there && known.sent.as_ref() == Some(self));
        let from = if new_there && !played && (!new_here || theirs.solved > self.solved) {
            theirs
        } else {
            self
        };
        Progress {
            level: from.level,
            solved,
            moves: from.moves.clone(),
        }
    }
}

/// What the phone last knew the PBX to have of a user's progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Known {
    /// The user whose progress it is (the PBX's id of the user), or ""
    /// if the phone's progress has never been to a PBX.
    pub user: String,
    /// The version of the PBX's entry, or 0 if it had none.
    pub version: u64,
    pub progress: Progress,
    /// What the phone sent to the PBX since, if the answer has not come:
    /// the game was left, or the PBX went out of reach, before it did.
    pub sent: Option<Progress>,
}

impl Known {
    /// Reads what [`Known::text`] wrote; text that makes no sense is a
    /// PBX that has nothing.
    pub fn parse(text: &str) -> Known {
        let version = field(text, "version").parse().unwrap_or(0);
        let sent = text
            .split_once("\nsent:\n")
            .map(|(_, sent)| Progress::parse(sent));
        Known {
            user: field(text, "user").to_string(),
            version,
            progress: if version == 0 {
                Progress::default()
            } else {
                Progress::parse(text)
            },
            sent,
        }
    }

    /// A line each for `user` and `version`, and the progress; then, if
    /// something was sent since, a line `sent:` and that progress.
    pub fn text(&self) -> String {
        let mut text = format!(
            "user={}\nversion={}\n{}",
            self.user,
            self.version,
            self.progress.text()
        );
        if let Some(sent) = &self.sent {
            text += "sent:\n";
            text += &sent.text();
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(level: usize, solved: usize, moves: &str) -> Progress {
        Progress {
            level,
            solved,
            moves: moves.into(),
        }
    }

    fn known(version: u64, progress: &Progress) -> Known {
        Known {
            user: "101".into(),
            version,
            progress: progress.clone(),
            sent: None,
        }
    }

    #[test]
    fn written_and_read() {
        let p = at(3, 5, "udlr");
        assert_eq!(Progress::parse(&p.text()), p);
        // Nonsense is the beginning, and no level is open that is not.
        assert_eq!(Progress::parse(""), Progress::default());
        assert_eq!(Progress::parse("solved=x\nlevel=5\n"), Progress::default());
        assert_eq!(
            Progress::parse("level=99\nsolved=3\nmoves=zz\n"),
            at(3, 3, "zz")
        );
        let all = LEVELS.len();
        assert_eq!(
            Progress::parse("level=99\nsolved=99\n"),
            at(all - 1, all, "")
        );
        // Too many steps are written as none.
        let long = at(1, 1, &"u".repeat(STEPS_KEPT + 1));
        assert_eq!(Progress::parse(&long.text()), at(1, 1, ""));

        let k = known(7, &p);
        assert_eq!(Known::parse(&k.text()), k);
        assert_eq!(Known::parse(""), Known::default());
        let k = Known {
            sent: Some(at(4, 5, "")),
            ..k
        };
        assert_eq!(Known::parse(&k.text()), k);
        // Without a version there is nothing the PBX is known to have.
        assert_eq!(
            Known::parse("user=101\nlevel=3\nsolved=5\n").progress,
            Progress::default()
        );
    }

    #[test]
    fn a_pbx_with_nothing_gets_the_phones() {
        let mine = at(2, 4, "ud");
        assert_eq!(mine.reconcile(&Known::default(), None, false), mine);
        // The PBX lost what it had.
        assert_eq!(mine.reconcile(&known(3, &mine), None, false), mine);
    }

    #[test]
    fn a_phone_with_nothing_new_gets_the_pbxs() {
        let theirs = at(6, 6, "rr");
        // A phone that was reset.
        let fresh = Progress::default();
        assert_eq!(
            fresh.reconcile(&Known::default(), Some((4, &theirs)), false),
            theirs
        );
        // The user played on another phone since this one wrote.
        let mine = at(2, 4, "ud");
        assert_eq!(
            mine.reconcile(&known(3, &mine), Some((4, &theirs)), false),
            theirs
        );
        // Another phone went back to an earlier level: no level closes.
        let back = at(1, 4, "l");
        let high = at(5, 5, "");
        assert_eq!(
            high.reconcile(&known(3, &high), Some((4, &back)), false),
            at(1, 5, "l")
        );
    }

    #[test]
    fn what_was_played_out_of_reach_goes_to_the_pbx() {
        let before = at(2, 2, "u");
        let mine = at(4, 4, "dd");
        // The PBX has what the phone last wrote.
        assert_eq!(
            mine.reconcile(&known(3, &before), Some((3, &before)), false),
            mine
        );
    }

    #[test]
    fn when_both_are_new_the_one_with_more_levels_solved_is_gone_on_with() {
        let before = at(2, 2, "u");
        let mine = at(4, 4, "dd");
        let k = known(3, &before);
        let further = at(9, 9, "l");
        assert_eq!(mine.reconcile(&k, Some((5, &further)), false), further);
        let behind = at(3, 3, "l");
        assert_eq!(mine.reconcile(&k, Some((5, &behind)), false), mine);
        // As many on both: the phone's.
        let level = at(1, 4, "rrr");
        assert_eq!(mine.reconcile(&k, Some((5, &level)), false), mine);
        // Behind in the level being played, but with more solved before.
        let mine = at(1, 3, "dd");
        let theirs = at(0, 6, "r");
        assert_eq!(mine.reconcile(&k, Some((5, &theirs)), false), theirs);
        // A phone whose progress has never been to a PBX.
        assert_eq!(
            mine.reconcile(&Known::default(), Some((5, &theirs)), false),
            theirs
        );
        assert_eq!(
            theirs.reconcile(&Known::default(), Some((5, &mine)), false),
            theirs
        );
    }

    #[test]
    fn what_was_sent_without_an_answer_is_nothing_new_once_the_pbx_has_another_entry() {
        let before = at(2, 2, "u");
        let mine = at(2, 2, "ud");
        let k = Known {
            sent: Some(mine.clone()),
            ..known(3, &before)
        };
        // It arrived, and the user then played on another phone, in the
        // same level or an earlier one.
        let theirs = at(1, 2, "l");
        assert_eq!(mine.reconcile(&k, Some((5, &theirs)), false), theirs);
        // It arrived, and nothing else happened.
        assert_eq!(mine.reconcile(&k, Some((4, &mine)), false), mine);
        // It did not arrive.
        assert_eq!(mine.reconcile(&k, Some((3, &before)), false), mine);
        // The player went on after sending it.
        let on = at(2, 2, "udd");
        assert_eq!(on.reconcile(&k, Some((5, &theirs)), false), on);
    }

    #[test]
    fn a_player_who_is_playing_stays_in_the_level() {
        let theirs = at(6, 6, "rr");
        let mine = at(0, 0, "r");
        // The solved levels open all the same.
        assert_eq!(
            mine.reconcile(&Known::default(), Some((4, &theirs)), true),
            at(0, 6, "r")
        );
        let fresh = Progress::default();
        assert_eq!(
            fresh.reconcile(&Known::default(), Some((4, &theirs)), true),
            at(0, 6, "")
        );
    }
}
