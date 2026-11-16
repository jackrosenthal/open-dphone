//! The alarm clock: the alarms, when each rings next, and what ringing,
//! snoozing, and dismissing do.  This is the whole of the logic, with the
//! clock, the time zone, and the calls handed in, so that it can be tested
//! without a phone; `mainrole::subsystems::alarms` runs it against the real
//! ones, keeps what [`Engine::saved`] gives in nvdata, and sounds the ringer.
//!
//! An alarm's time is a time of day in the phone's time zone.  On the day
//! the clocks go forward, a time that does not exist (02:30) rings as late
//! after midnight as on any other day (at 03:30 by the new time); on the
//! day they go back, a time that comes twice rings the first time only.
//!
//! No model has a clock chip, and a phone can be off or in a call when an
//! alarm is due.  An alarm that could not ring on time rings as soon as it
//! can, if that is no more than [`LATE_SECS`] late, and is skipped
//! otherwise (a one-time alarm that is skipped is switched off, as one that
//! rang is).

use dphone_state::{Alarm, AlarmRinging, Alarms};
use serde::{Deserialize, Serialize};

/// How long a snooze lasts.
pub const SNOOZE_SECS: u64 = 10 * 60;
/// How long an alarm rings before it snoozes itself.
pub const RING_SECS: u64 = 60;
/// How many times in a row an alarm snoozes itself before it gives up.
pub const UNATTENDED_SNOOZES: u8 = 3;
/// The latest an alarm still rings after its time.
pub const LATE_SECS: u64 = 30 * 60;

/// One alarm as it is kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    #[serde(flatten)]
    alarm: Alarm,
    /// Occurrences count from here: the time the alarm was last set, or
    /// its last occurrence that rang.
    #[serde(default)]
    since: u64,
}

/// An alarm that is to ring (again) at `until`: a snoozed one, or the one
/// that is ringing (`until` is then when it began, so that a phone that
/// restarts meanwhile rings it again).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Pending {
    id: u32,
    until: u64,
    /// How many times in a row it snoozed itself.
    #[serde(default)]
    unattended: u8,
}

/// What is kept across restarts (`/nvdata/alarms.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    alarms: Vec<Entry>,
    /// The last identifier given out.
    #[serde(default)]
    last_id: u32,
    #[serde(default)]
    pending: Option<Pending>,
}

struct Ringing {
    id: u32,
    started: u64,
    unattended: u8,
}

/// What the ringer is to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sound {
    /// Ring with this ringtone until told otherwise.
    Ring(String),
    Silence,
}

/// What a call into the engine asks of its caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// [`Engine::saved`] changed and is to be written.
    pub save: bool,
    pub sound: Option<Sound>,
}

/// Seconds east of UTC at a Unix time.
pub type Offset<'a> = &'a dyn Fn(i64) -> i32;

pub struct Engine {
    saved: Saved,
    ringing: Option<Ringing>,
    effects: Effects,
    /// The minute of the last look at every alarm (`None`: look again).
    looked: Option<u64>,
}

/// The Unix time at which the clock of a zone shows `local` (seconds since
/// the epoch as the local clock counts them).
fn resolve(local: i64, offset: Offset) -> i64 {
    // The offsets in force a day before and a day after: equal, unless the
    // clocks change in between.
    let (before, after) = (offset(local - 86_400) as i64, offset(local + 86_400) as i64);
    let (a, b) = (local - before, local - after);
    if a == b {
        return a;
    }
    match (offset(a) as i64 == before, offset(b) as i64 == after) {
        // The local time comes twice: the first of them.
        (true, true) => a.min(b),
        (true, false) => a,
        (false, true) => b,
        // The local time is skipped: as long after midnight as ever.
        (false, false) => a.max(b),
    }
}

/// The first time after `after` at which an alarm set for `hour`:`minute`
/// on `days` (bit 0 Sunday; 0 for any day) is due.
pub fn next_occurrence(hour: u8, minute: u8, days: u8, after: u64, offset: Offset) -> u64 {
    let after = after as i64;
    let today = (after + offset(after) as i64).div_euclid(86_400);
    let of_day = i64::from(hour) * 3600 + i64::from(minute) * 60;
    // The day before too: where the clocks just went back, `after` can be
    // on a later local date than a time that is still to come.
    for day in today - 1..=today + 8 {
        // 1970-01-01 was a Thursday.
        let weekday = (day + 4).rem_euclid(7);
        if days != 0 && days & (1 << weekday) == 0 {
            continue;
        }
        let at = resolve(day * 86_400 + of_day, offset);
        if at > after {
            return at as u64;
        }
    }
    // Not reached: nine days hold every day of the week.
    (after + 9 * 86_400) as u64
}

impl Engine {
    /// The alarm clock from what was kept (`None` or unreadable text: no
    /// alarms).
    pub fn load(text: Option<&str>) -> Engine {
        let saved = match text.map(serde_json::from_str::<Saved>) {
            Some(Ok(saved)) => saved,
            Some(Err(e)) => {
                log::warn!("alarms: the saved alarms cannot be read ({e}); starting with none");
                Saved::default()
            }
            None => Saved::default(),
        };
        Engine { saved, ringing: None, effects: Effects::default(), looked: None }
    }

    /// What to keep across restarts.
    pub fn saved(&self) -> String {
        serde_json::to_string_pretty(&self.saved).unwrap_or_default()
    }

    /// What was asked of the caller since the last call of this.
    pub fn take_effects(&mut self) -> Effects {
        std::mem::take(&mut self.effects)
    }

    fn entry(&self, id: u32) -> Option<&Entry> {
        self.saved.alarms.iter().find(|e| e.alarm.id == id)
    }

    /// Adds an alarm (`alarm.id` 0) or changes one; returns its identifier.
    /// `tones` are the ringtones the phone has; an empty tone is the first
    /// of them.
    pub fn set(&mut self, mut alarm: Alarm, now: u64, tones: &[String]) -> Result<u32, String> {
        if alarm.hour > 23 || alarm.minute > 59 {
            return Err("the time must be between 00:00 and 23:59".into());
        }
        if alarm.days > Alarm::EVERY_DAY {
            return Err("the days are not days of the week".into());
        }
        if alarm.tone.is_empty() {
            alarm.tone = tones.first().cloned().unwrap_or_default();
        }
        match tones.iter().find(|t| t.eq_ignore_ascii_case(&alarm.tone)) {
            Some(t) => alarm.tone = t.clone(),
            None => return Err(format!("the phone has no ringtone named {:?}", alarm.tone)),
        }
        let id = if alarm.id == 0 {
            if self.saved.alarms.len() >= Alarms::MAX {
                return Err(format!("the phone keeps at most {} alarms", Alarms::MAX));
            }
            self.saved.last_id = self.saved.last_id.wrapping_add(1).max(1);
            alarm.id = self.saved.last_id;
            self.saved.alarms.push(Entry { alarm, since: now });
            self.saved.last_id
        } else {
            let id = alarm.id;
            let Some(e) = self.saved.alarms.iter_mut().find(|e| e.alarm.id == id) else {
                return Err("no such alarm".into());
            };
            if e.alarm == alarm {
                return Ok(id);
            }
            *e = Entry { alarm, since: now };
            // What was ringing or snoozed was the alarm as it was.
            self.forget(id);
            id
        };
        self.changed();
        Ok(id)
    }

    pub fn remove(&mut self, id: u32) -> Result<(), String> {
        let before = self.saved.alarms.len();
        self.saved.alarms.retain(|e| e.alarm.id != id);
        if self.saved.alarms.len() == before {
            return Err("no such alarm".into());
        }
        self.forget(id);
        self.changed();
        Ok(())
    }

    fn changed(&mut self) {
        self.effects.save = true;
        self.looked = None;
    }

    /// Ends the ringing or the snooze of an alarm that changed or went.
    fn forget(&mut self, id: u32) {
        if self.ringing.as_ref().is_some_and(|r| r.id == id) {
            self.ringing = None;
            self.effects.sound = Some(Sound::Silence);
        }
        if self.saved.pending.is_some_and(|p| p.id == id) {
            self.saved.pending = None;
        }
    }

    /// Silences the ringing alarm until [`SNOOZE_SECS`] from now.  One
    /// that snoozes itself (`by_user` false) too often in a row gives up.
    pub fn snooze(&mut self, now: u64, by_user: bool) -> bool {
        let Some(r) = self.ringing.take() else { return false };
        let unattended = if by_user { 0 } else { r.unattended + 1 };
        self.saved.pending = (unattended <= UNATTENDED_SNOOZES).then_some(Pending {
            id: r.id,
            until: now + SNOOZE_SECS,
            unattended,
        });
        self.effects.sound = Some(Sound::Silence);
        self.changed();
        true
    }

    /// Ends the ringing alarm, or the snoozed one.
    pub fn dismiss(&mut self) -> bool {
        let ringing = self.ringing.take().is_some();
        if ringing {
            self.effects.sound = Some(Sound::Silence);
        }
        if !ringing && self.saved.pending.is_none() {
            return false;
        }
        self.saved.pending = None;
        self.changed();
        true
    }

    fn ring(&mut self, id: u32, now: u64, unattended: u8) {
        let Some(e) = self.entry(id) else { return };
        self.effects.sound = Some(Sound::Ring(e.alarm.tone.clone()));
        self.ringing = Some(Ringing { id, started: now, unattended });
        self.saved.pending = Some(Pending { id, until: now, unattended });
        self.effects.save = true;
    }

    /// Where the occurrences of an alarm are looked for from: not so far
    /// back that one would be too late to ring.
    fn from(e: &Entry, now: u64) -> u64 {
        e.since.max(now.saturating_sub(LATE_SECS))
    }

    /// Lets time pass: `now` is the wall clock, and `busy` is whether the
    /// phone has a call (an alarm does not ring into one).  To be called
    /// about once a second, and after every change.
    pub fn tick(&mut self, now: u64, busy: bool, offset: Offset) {
        if let Some(r) = &self.ringing {
            if busy || now >= r.started + RING_SECS {
                self.snooze(now, false);
            }
            return;
        }
        if busy {
            self.looked = None;
            return;
        }
        // The clock was set back: a snooze is no longer than a snooze.
        if let Some(p) = &mut self.saved.pending
            && p.until > now + SNOOZE_SECS
        {
            p.until = now + SNOOZE_SECS;
            self.effects.save = true;
        }
        if let Some(p) = self.saved.pending
            && p.until <= now
        {
            if now - p.until <= LATE_SECS && self.entry(p.id).is_some() {
                self.ring(p.id, now, p.unattended);
                return;
            }
            self.saved.pending = None;
            self.changed();
        }
        // The alarms are due on whole minutes.
        if self.looked == Some(now / 60) {
            return;
        }
        self.looked = Some(now / 60);

        let mut due: Vec<(u64, u32)> = Vec::new();
        for e in &mut self.saved.alarms {
            // The clock was set back: count from the new time.
            if e.since > now {
                e.since = now;
                self.effects.save = true;
            }
            if !e.alarm.enabled {
                continue;
            }
            let a = &e.alarm;
            // A one-time alarm whose time passed too long ago to ring.
            if a.days == 0 && next_occurrence(a.hour, a.minute, 0, e.since, offset) + LATE_SECS < now {
                e.alarm.enabled = false;
                self.effects.save = true;
                continue;
            }
            let at = next_occurrence(a.hour, a.minute, a.days, Self::from(e, now), offset);
            if at <= now {
                due.push((at, a.id));
                e.since = at;
                if a.days == 0 {
                    e.alarm.enabled = false;
                }
            }
        }
        // Alarms due together ring as one: the earliest of them.
        if let Some(&(_, id)) = due.iter().min() {
            self.ring(id, now, 0);
        }
    }

    /// The state to publish.
    pub fn state(&self, now: u64, tones: &[String], offset: Offset) -> Alarms {
        let ringing = self.ringing.as_ref().and_then(|r| self.entry(r.id)).map(|e| AlarmRinging {
            id: e.alarm.id,
            hour: e.alarm.hour,
            minute: e.alarm.minute,
        });
        let snoozed = match (&ringing, self.saved.pending) {
            (None, Some(p)) => Some((p.id, p.until)),
            _ => None,
        };
        let next = self
            .saved
            .alarms
            .iter()
            .filter(|e| e.alarm.enabled)
            .map(|e| {
                let a = &e.alarm;
                (next_occurrence(a.hour, a.minute, a.days, Self::from(e, now), offset), a.id)
            })
            .chain(snoozed.map(|(id, until)| (until, id)))
            .min()
            .map(|(at, id)| (id, at));
        let upcoming = ringing.is_some() || next.is_some_and(|(_, at)| at <= now + Alarms::UPCOMING_SECS);
        Alarms {
            alarms: self.saved.alarms.iter().map(|e| e.alarm.clone()).collect(),
            tones: tones.to_vec(),
            icon: upcoming,
            next,
            ringing,
            snoozed,
            snooze_secs: SNOOZE_SECS as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::days_from_civil;
    use crate::tz::Zone;

    const SUN: u8 = 1;
    const MON: u8 = 1 << 1;
    const WED: u8 = 1 << 3;
    const FRI: u8 = 1 << 5;

    /// A Unix time from a date and time in UTC.
    fn utc(y: i64, mo: u32, d: u32, h: u64, mi: u64) -> u64 {
        days_from_civil(y, mo, d) as u64 * 86_400 + h * 3600 + mi * 60
    }

    fn zone(name: &str) -> impl Fn(i64) -> i32 + 'static {
        let z = Zone::new(name);
        move |t| z.offset_at(t)
    }

    fn tones() -> Vec<String> {
        vec!["Marimba".to_string(), "Classic".to_string()]
    }

    fn alarm(hour: u8, minute: u8, days: u8) -> Alarm {
        Alarm { id: 0, hour, minute, days, tone: "Classic".into(), enabled: true }
    }

    /// An engine with a clock that the test moves.
    struct Rig {
        e: Engine,
        now: u64,
        busy: bool,
        offset: Box<dyn Fn(i64) -> i32>,
        /// Every sound asked for, in order.
        sounds: Vec<Sound>,
        saves: u32,
    }

    impl Rig {
        fn new(zone_name: &'static str, now: u64) -> Rig {
            Rig { e: Engine::load(None), now, busy: false, offset: Box::new(zone(zone_name)), sounds: Vec::new(), saves: 0 }
        }
        fn collect(&mut self) {
            let fx = self.e.take_effects();
            self.sounds.extend(fx.sound);
            self.saves += u32::from(fx.save);
        }
        fn add(&mut self, a: Alarm) -> u32 {
            let id = self.e.set(a, self.now, &tones()).unwrap();
            self.collect();
            id
        }
        /// Runs the clock to `until`, a second at a time.
        fn run_to(&mut self, until: u64) {
            while self.now < until {
                self.now += 1;
                self.e.tick(self.now, self.busy, &*self.offset);
                self.collect();
            }
        }
        fn run(&mut self, secs: u64) {
            self.run_to(self.now + secs);
        }
        fn state(&self) -> Alarms {
            self.e.state(self.now, &tones(), &*self.offset)
        }
        fn ringing(&self) -> Option<u32> {
            self.state().ringing.map(|r| r.id)
        }
        fn snooze(&mut self) {
            assert!(self.e.snooze(self.now, true));
            self.collect();
        }
        fn dismiss(&mut self) {
            assert!(self.e.dismiss());
            self.collect();
        }
        /// The phone restarts: everything but what was saved is lost, and
        /// the clock reads `now` when it is up again.
        fn restart(&mut self, now: u64) {
            self.e = Engine::load(Some(&self.e.saved()));
            self.now = now;
        }
    }

    #[test]
    fn the_next_occurrence_is_later_today_or_tomorrow() {
        let la = zone("America/Los_Angeles");
        // Monday 2026-11-16, 07:00 PST (15:00 UTC).
        let now = utc(2026, 11, 16, 15, 0);
        assert_eq!(next_occurrence(8, 0, 0, now, &la), utc(2026, 11, 16, 16, 0));
        assert_eq!(next_occurrence(6, 30, 0, now, &la), utc(2026, 11, 17, 14, 30));
        // Exactly at its time, an alarm's next occurrence is tomorrow's.
        assert_eq!(next_occurrence(7, 0, 0, now, &la), utc(2026, 11, 17, 15, 0));
        assert_eq!(next_occurrence(7, 0, Alarm::EVERY_DAY, now - 1, &la), now);
    }

    #[test]
    fn a_weekly_alarm_rings_on_its_days_across_weeks() {
        let la = zone("America/Los_Angeles");
        // Monday 2026-11-16, 07:00 PST: Monday at 08:00 is today.
        let mut t = utc(2026, 11, 16, 15, 0);
        let mut seen = Vec::new();
        for _ in 0..3 {
            t = next_occurrence(8, 0, MON, t, &la);
            seen.push(t);
        }
        assert_eq!(seen, [utc(2026, 11, 16, 16, 0), utc(2026, 11, 23, 16, 0), utc(2026, 11, 30, 16, 0)]);
        // Monday, Wednesday, and Friday, from Monday after the alarm.
        let mut t = utc(2026, 11, 16, 17, 0);
        let mut seen = Vec::new();
        for _ in 0..4 {
            t = next_occurrence(8, 0, MON | WED | FRI, t, &la);
            seen.push(t);
        }
        assert_eq!(
            seen,
            [utc(2026, 11, 18, 16, 0), utc(2026, 11, 20, 16, 0), utc(2026, 11, 23, 16, 0), utc(2026, 11, 25, 16, 0)]
        );
        // The day is the local one: Sunday 23:30 in Los Angeles is Monday in UTC.
        assert_eq!(next_occurrence(23, 30, SUN, utc(2026, 11, 16, 15, 0), &la), utc(2026, 11, 23, 7, 30));
        // East of UTC, likewise: Monday 06:00 in Sydney is Sunday in UTC.
        let sydney = zone("Australia/Sydney");
        assert_eq!(next_occurrence(6, 0, MON, utc(2026, 11, 16, 15, 0), &sydney), utc(2026, 11, 22, 19, 0));
    }

    #[test]
    fn an_alarm_keeps_its_local_time_when_the_clocks_change() {
        let la = zone("America/Los_Angeles");
        // Clocks go forward on Sunday 2027-03-14 at 02:00 PST (10:00 UTC).
        let sat = next_occurrence(8, 0, Alarm::EVERY_DAY, utc(2027, 3, 13, 0, 0), &la);
        assert_eq!(sat, utc(2027, 3, 13, 16, 0));
        let sun = next_occurrence(8, 0, Alarm::EVERY_DAY, sat, &la);
        assert_eq!(sun, utc(2027, 3, 14, 15, 0));
        // They go back on Sunday 2027-11-07 at 02:00 PDT (09:00 UTC).
        let sat = next_occurrence(8, 0, Alarm::EVERY_DAY, utc(2027, 11, 6, 0, 0), &la);
        assert_eq!(sat, utc(2027, 11, 6, 15, 0));
        assert_eq!(next_occurrence(8, 0, Alarm::EVERY_DAY, sat, &la), utc(2027, 11, 7, 16, 0));
        // Europe changes at 01:00 UTC, on other days.
        let berlin = zone("Europe/Berlin");
        let sat = next_occurrence(7, 0, Alarm::EVERY_DAY, utc(2027, 3, 27, 0, 0), &berlin);
        assert_eq!(sat, utc(2027, 3, 27, 6, 0));
        assert_eq!(next_occurrence(7, 0, Alarm::EVERY_DAY, sat, &berlin), utc(2027, 3, 28, 5, 0));
        // South of the equator the clocks go forward in October.
        let sydney = zone("Australia/Sydney");
        let sat = next_occurrence(7, 0, Alarm::EVERY_DAY, utc(2027, 10, 1, 14, 0), &sydney);
        assert_eq!(sat, utc(2027, 10, 1, 21, 0));
        assert_eq!(next_occurrence(7, 0, Alarm::EVERY_DAY, sat, &sydney), utc(2027, 10, 2, 20, 0));
    }

    #[test]
    fn a_time_the_clocks_skip_rings_an_hour_later_by_the_new_time() {
        let la = zone("America/Los_Angeles");
        // 02:30 does not exist on 2027-03-14: 03:30 PDT (10:30 UTC).
        let sat = next_occurrence(2, 30, Alarm::EVERY_DAY, utc(2027, 3, 13, 0, 0), &la);
        assert_eq!(sat, utc(2027, 3, 13, 10, 30));
        let sun = next_occurrence(2, 30, Alarm::EVERY_DAY, sat, &la);
        assert_eq!(sun, utc(2027, 3, 14, 10, 30));
        assert_eq!(next_occurrence(2, 30, Alarm::EVERY_DAY, sun, &la), utc(2027, 3, 15, 9, 30));
    }

    #[test]
    fn a_time_the_clocks_show_twice_rings_the_first_time_only() {
        let la = zone("America/Los_Angeles");
        // 01:30 comes twice on 2027-11-07: at 08:30 UTC (PDT) and 09:30 UTC (PST).
        let first = next_occurrence(1, 30, Alarm::EVERY_DAY, utc(2027, 11, 7, 0, 0), &la);
        assert_eq!(first, utc(2027, 11, 7, 8, 30));
        assert_eq!(next_occurrence(1, 30, Alarm::EVERY_DAY, first, &la), utc(2027, 11, 8, 9, 30));
        // Between the two, it is not due again that day either.
        assert_eq!(next_occurrence(1, 30, Alarm::EVERY_DAY, utc(2027, 11, 7, 9, 0), &la), utc(2027, 11, 8, 9, 30));
    }

    #[test]
    fn a_one_time_alarm_rings_once_and_is_switched_off() {
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 0));
        let id = r.add(alarm(8, 0, 0));
        assert_eq!(r.state().next, Some((id, utc(2026, 11, 16, 16, 0))));
        r.run_to(utc(2026, 11, 16, 16, 0) - 1);
        assert_eq!(r.ringing(), None);
        r.run(1);
        assert_eq!(r.ringing(), Some(id));
        assert_eq!(r.sounds, [Sound::Ring("Classic".into())]);
        r.dismiss();
        assert_eq!(r.sounds.last(), Some(&Sound::Silence));
        let st = r.state();
        assert!(!st.alarms[0].enabled);
        assert_eq!((st.next, st.ringing, st.snoozed, st.icon), (None, None, None, false));
        // Not the next day either.
        r.run(2 * 86_400);
        assert_eq!(r.sounds.len(), 2);
    }

    #[test]
    fn a_recurring_alarm_rings_every_time() {
        // Sunday 2026-11-15, 12:00 PST.
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 15, 20, 0));
        let id = r.add(alarm(8, 0, MON | WED));
        let mut rang = Vec::new();
        while r.now < utc(2026, 11, 24, 0, 0) {
            r.run(1);
            if r.ringing().is_some() {
                rang.push(r.now);
                r.dismiss();
            }
        }
        assert_eq!(rang, [utc(2026, 11, 16, 16, 0), utc(2026, 11, 18, 16, 0), utc(2026, 11, 23, 16, 0)]);
        assert!(r.state().alarms[0].enabled);
        assert_eq!(r.state().next, Some((id, utc(2026, 11, 25, 16, 0))));
    }

    #[test]
    fn snooze_rings_again_after_ten_minutes_and_dismiss_ends_it() {
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 59));
        let id = r.add(alarm(8, 0, Alarm::EVERY_DAY));
        r.run(60);
        assert_eq!(r.ringing(), Some(id));
        r.run(5);
        r.snooze();
        let again = r.now + SNOOZE_SECS;
        let st = r.state();
        assert_eq!((st.ringing, st.snoozed, st.next), (None, Some((id, again)), Some((id, again))));
        assert!(st.icon);
        r.run_to(again - 1);
        assert_eq!(r.ringing(), None);
        r.run(1);
        assert_eq!(r.ringing(), Some(id));
        // Snoozed again and then dismissed while snoozed: it stays quiet
        // until tomorrow.
        r.snooze();
        r.dismiss();
        assert_eq!(r.state().snoozed, None);
        assert_eq!(r.state().next, Some((id, utc(2026, 11, 17, 16, 0))));
        r.run(3600);
        assert_eq!(
            r.sounds,
            [Sound::Ring("Classic".into()), Sound::Silence, Sound::Ring("Classic".into()), Sound::Silence]
        );
    }

    #[test]
    fn an_alarm_nobody_attends_to_snoozes_itself_and_then_gives_up() {
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 59));
        r.add(alarm(8, 0, 0));
        let start = utc(2026, 11, 16, 16, 0);
        let mut rings = Vec::new();
        let mut was = false;
        while r.now < start + 3 * 3600 {
            r.run(1);
            let is = r.ringing().is_some();
            if is != was {
                rings.push((r.now - start, is));
                was = is;
            }
        }
        // A minute of ringing, ten minutes of quiet, four times in all.
        let period = RING_SECS + SNOOZE_SECS;
        let want: Vec<(u64, bool)> =
            (0..=u64::from(UNATTENDED_SNOOZES)).flat_map(|i| [(i * period, true), (i * period + RING_SECS, false)]).collect();
        assert_eq!(rings, want);
        assert_eq!(r.state().snoozed, None);
    }

    #[test]
    fn an_alarm_does_not_ring_into_a_call() {
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 59));
        let id = r.add(alarm(8, 0, Alarm::EVERY_DAY));
        r.busy = true;
        r.run(10 * 60);
        assert_eq!(r.ringing(), None);
        assert!(r.sounds.is_empty());
        // The call ends nine minutes late: it rings now.
        r.busy = false;
        r.run(1);
        assert_eq!(r.ringing(), Some(id));
        // A call that comes while it rings snoozes it.
        r.busy = true;
        r.run(1);
        assert_eq!(r.ringing(), None);
        assert_eq!(r.state().snoozed, Some((id, r.now + SNOOZE_SECS)));
        // A call that lasts past the time by more than half an hour: skipped.
        r.dismiss();
        r.run_to(utc(2026, 11, 17, 16, 0) + LATE_SECS + 1);
        r.busy = false;
        r.run(120);
        assert_eq!(r.ringing(), None);
        assert_eq!(r.state().next, Some((id, utc(2026, 11, 18, 16, 0))));
    }

    #[test]
    fn alarms_and_a_snooze_outlast_a_restart() {
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 0));
        let daily = r.add(alarm(8, 0, Alarm::EVERY_DAY));
        let once = r.add(Alarm { tone: "marimba".into(), ..alarm(9, 0, 0) });
        r.collect();
        r.restart(utc(2026, 11, 16, 15, 30));
        let st = r.state();
        assert_eq!(st.alarms.len(), 2);
        assert_eq!(st.alarms[1], Alarm { id: once, hour: 9, minute: 0, days: 0, tone: "Marimba".into(), enabled: true });
        // It rings after the restart, without anything else being done.
        r.run_to(utc(2026, 11, 16, 16, 0));
        assert_eq!(r.ringing(), Some(daily));
        // Snoozed, and the phone restarts during the snooze.
        r.snooze();
        let again = r.now + SNOOZE_SECS;
        r.restart(r.now + 120);
        assert_eq!(r.state().snoozed, Some((daily, again)));
        r.run_to(again);
        assert_eq!(r.ringing(), Some(daily));
        // The phone restarts while it rings: it rings again when it is up.
        r.restart(r.now + 90);
        r.run(1);
        assert_eq!(r.ringing(), Some(daily));
        r.dismiss();
        // A new alarm does not get the identifier of an old one.
        r.e.remove(once).unwrap();
        r.restart(r.now);
        assert!(r.add(alarm(10, 0, 0)) > once);
    }

    #[test]
    fn an_alarm_due_while_the_phone_was_off_rings_if_it_is_not_too_late() {
        // Off from 07:50 to 08:20 PST: the 08:00 alarm rings at once.
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 50));
        let daily = r.add(alarm(8, 0, Alarm::EVERY_DAY));
        let once = r.add(alarm(8, 10, 0));
        r.restart(utc(2026, 11, 16, 16, 20));
        r.run(1);
        // The two were due together: one ring, the earlier alarm's.
        assert_eq!(r.ringing(), Some(daily));
        r.dismiss();
        r.run(120);
        assert_eq!(r.sounds.len(), 2);
        assert!(!r.state().get(once).unwrap().enabled);

        // Off from Monday evening to Wednesday 08:45: Tuesday's and
        // Wednesday's are skipped, and a one-time alarm is switched off.
        let once = r.add(alarm(7, 0, 0));
        r.restart(utc(2026, 11, 18, 16, 45));
        r.run(600);
        assert_eq!(r.sounds.len(), 2);
        assert!(!r.state().get(once).unwrap().enabled);
        assert_eq!(r.state().next, Some((daily, utc(2026, 11, 19, 16, 0))));
    }

    #[test]
    fn the_clock_being_set_does_not_ring_old_alarms() {
        // The phone starts with a clock that is two days behind.
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 14, 15, 0));
        let id = r.add(alarm(8, 0, Alarm::EVERY_DAY));
        r.now = utc(2026, 11, 16, 17, 0);
        r.run(60);
        assert_eq!(r.ringing(), None);
        assert_eq!(r.state().next, Some((id, utc(2026, 11, 17, 16, 0))));
        // Set back by a day: the alarm rings at its time by the new clock.
        r.run_to(utc(2026, 11, 17, 16, 0));
        r.dismiss();
        r.now = utc(2026, 11, 16, 15, 0);
        r.run_to(utc(2026, 11, 16, 16, 0));
        assert_eq!(r.ringing(), Some(id));
    }

    #[test]
    fn the_icon_shows_for_an_alarm_within_a_day() {
        // Monday 07:00 PST.
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 0));
        assert!(!r.state().icon);
        let id = r.add(alarm(8, 0, WED));
        assert!(!r.state().icon);
        r.run_to(utc(2026, 11, 17, 16, 0) - 1);
        assert!(!r.state().icon);
        r.run(1);
        assert!(r.state().icon);
        // Switched off, there is no upcoming alarm.
        r.e.set(Alarm { id, enabled: false, ..alarm(8, 0, WED) }, r.now, &tones()).unwrap();
        assert!(!r.state().icon);
        assert_eq!(r.state().next, None);
        r.e.set(Alarm { id, ..alarm(8, 0, WED) }, r.now, &tones()).unwrap();
        assert!(r.state().icon);
    }

    #[test]
    fn a_file_with_the_former_icon_setting_loads() {
        let mut r = Rig::new("America/Los_Angeles", utc(2026, 11, 16, 15, 0));
        let id = r.add(alarm(8, 0, Alarm::EVERY_DAY));
        // As the firmware that had the setting wrote the file.
        let mut saved: serde_json::Value = serde_json::from_str(&r.e.saved()).unwrap();
        saved["show_icon"] = false.into();
        r.e = Engine::load(Some(&saved.to_string()));
        let st = r.state();
        assert_eq!(st.next, Some((id, utc(2026, 11, 16, 16, 0))));
        assert!(st.icon);
    }

    #[test]
    fn what_cannot_be_an_alarm_is_refused() {
        let mut e = Engine::load(None);
        let t = tones();
        assert!(e.set(alarm(24, 0, 0), 0, &t).is_err());
        assert!(e.set(alarm(0, 60, 0), 0, &t).is_err());
        assert!(e.set(alarm(0, 0, 0x80), 0, &t).is_err());
        assert!(e.set(Alarm { tone: "Nope".into(), ..alarm(1, 0, 0) }, 0, &t).is_err());
        assert!(e.set(Alarm { id: 7, ..alarm(1, 0, 0) }, 0, &t).is_err());
        assert!(e.remove(7).is_err());
        // An alarm without a tone gets the phone's first.
        let id = e.set(Alarm { tone: String::new(), ..alarm(1, 0, 0) }, 0, &t).unwrap();
        assert_eq!(e.state(0, &t, &|_| 0).get(id).unwrap().tone, "Marimba");
        for _ in 1..Alarms::MAX {
            e.set(alarm(1, 0, 0), 0, &t).unwrap();
        }
        assert!(e.set(alarm(1, 0, 0), 0, &t).is_err());
        // Changing one is still possible, and removing one makes room.
        e.set(Alarm { id, ..alarm(2, 0, MON) }, 0, &t).unwrap();
        e.remove(id).unwrap();
        e.set(alarm(1, 0, 0), 0, &t).unwrap();
        // Text that is not the saved alarms is no alarms.
        assert!(Engine::load(Some("{ not json")).state(0, &t, &|_| 0).alarms.is_empty());
    }

    #[test]
    fn changing_or_removing_the_ringing_alarm_silences_it() {
        let mut r = Rig::new("UTC", utc(2026, 11, 16, 7, 59));
        let id = r.add(alarm(8, 0, Alarm::EVERY_DAY));
        r.run(60);
        assert_eq!(r.ringing(), Some(id));
        r.e.remove(id).unwrap();
        r.collect();
        assert_eq!(r.sounds.last(), Some(&Sound::Silence));
        assert_eq!((r.ringing(), r.state().snoozed), (None, None));
        r.run(3600);
        assert_eq!(r.sounds.len(), 2);
    }
}
