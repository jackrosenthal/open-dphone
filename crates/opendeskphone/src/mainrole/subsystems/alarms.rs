//! The alarm clock: [`crate::alarms::Engine`] run against the wall clock, the
//! configured time zone, and the calls.  It keeps the alarms in
//! `/nvdata/opendeskphone/alarms.txtpb`, rings them on the ringer, publishes
//! [`opendeskphone_state::Alarms`] (which the UI shows the ringing alarm and
//! the idle screen's icon from), and answers the `Alarms` commands that the
//! Alarms app, the web interface, and the ringing alarm's softkeys send.
//! Nothing here depends on an app running.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opendeskphone_config::Config;
use opendeskphone_state::{Alarm, Alarms, Calls, Command, CommandError, Domain, Reply, Topic};
use log::{info, warn};
use tokio::sync::Notify;

use super::audio::AudioService;
use super::super::Context;
use crate::alarms::{Engine, Sound};
use crate::tz::Zone;

/// The file in nvdata that holds the alarms.
pub const FILE: &str = opendeskphone_config::saved::ALARMS_FILE;

struct Shared {
    zone: Mutex<Zone>,
    /// Something outside changed (the configuration): look again now.
    changed: Notify,
}

/// Handle for configuration changes.
#[derive(Clone)]
pub struct AlarmService {
    shared: Arc<Shared>,
}

impl AlarmService {
    /// Takes the time zone from the merged configuration.
    pub fn apply_config(&self, config: &Config) {
        *self.shared.zone.lock().unwrap() = Zone::new(&config.time().time_zone);
        self.shared.changed.notify_one();
    }
}

struct Runner {
    ctx: Context,
    audio: AudioService,
    shared: Arc<Shared>,
    engine: Engine,
    path: PathBuf,
    tones: Vec<String>,
}

impl Runner {
    fn command(&mut self, command: Command, now: u64) -> Result<Reply, CommandError> {
        match command {
            Command::SetAlarm { alarm } => {
                self.engine.set(alarm, now, &self.tones).map(|id| Reply::Id(id.into())).map_err(CommandError::Invalid)
            }
            Command::RemoveAlarm { id } => self.engine.remove(id).map(|()| Reply::Done).map_err(CommandError::Invalid),
            Command::SnoozeAlarm => match self.engine.snooze(now, true) {
                true => Ok(Reply::Done),
                false => Err(CommandError::Invalid("no alarm is ringing".into())),
            },
            Command::DismissAlarm => match self.engine.dismiss() {
                true => Ok(Reply::Done),
                false => Err(CommandError::Invalid("no alarm is ringing or snoozed".into())),
            },
            other => Err(CommandError::Invalid(format!("not an alarm command: {other:?}"))),
        }
    }

    /// Lets the engine see the time, sounds or silences the ringer as it
    /// asks, and publishes.  Returns whether the alarms are to be written.
    fn advance(&mut self, now: u64) -> bool {
        let zone = *self.shared.zone.lock().unwrap();
        let offset = move |t: i64| zone.offset_at(t);
        let busy = !self.ctx.store.read::<Calls, _>(|c| c.is_idle());
        self.engine.tick(now, busy, &offset);
        let effects = self.engine.take_effects();
        match effects.sound {
            Some(Sound::Ring(tone)) => {
                info!("alarms: an alarm rings ({tone})");
                self.audio.alarm(Some(&tone));
            }
            Some(Sound::Silence) => self.audio.alarm(None),
            None => {}
        }
        let state = self.engine.state(now, &self.tones, &offset);
        if *self.ctx.store.get::<Alarms>() != state {
            self.ctx.store.set(state);
        }
        effects.save
    }

    async fn save(&self) {
        let (path, text) = (self.path.clone(), self.engine.saved());
        let written = tokio::task::spawn_blocking(move || crate::paths::write_atomic(&path, text.as_bytes(), 0o644)).await;
        if let Ok(Err(e)) = written {
            warn!("alarms: cannot write {}: {e}", self.path.display());
        }
    }

    async fn run(mut self) {
        let commands = self.ctx.store.handle(Domain::Alarms);
        // A call that begins silences a ringing alarm at once.
        let sub = self.ctx.store.subscribe_events([Topic::Calls]);
        let mut second = tokio::time::interval(Duration::from_secs(1));
        second.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if self.advance(opendeskphone_state::clock::unix_secs()) {
                self.save().await;
            }
            let shared = self.shared.clone();
            tokio::select! {
                envelope = commands.next() => {
                    let Some(envelope) = envelope else { return };
                    let (command, responder) = envelope.split();
                    let now = opendeskphone_state::clock::unix_secs();
                    let result = self.command(command, now);
                    // Published before the answer: who asked sees the
                    // change in the state when the answer is there.
                    let save = self.advance(now);
                    responder.reply(result);
                    if save {
                        self.save().await;
                    }
                }
                _ = second.tick() => {}
                _ = sub.changed() => {}
                _ = shared.changed.notified() => {}
            }
        }
    }
}

/// Alarm clock
///
/// Without a subcommand: the alarms and what the clock is doing.  The
/// others print the same after they have acted.
#[derive(Debug, clap::Parser)]
#[command(name = "alarms")]
pub struct AlarmsCommand {
    #[command(subcommand)]
    action: Option<AlarmsAction>,
}

#[derive(Debug, clap::Subcommand)]
enum AlarmsAction {
    /// Show the alarms
    List,
    /// Add an alarm
    Add {
        /// The time of day, HH:MM (24-hour)
        time: String,
        /// The days it rings on, e.g., mon,wed,fri; one time without
        #[arg(long)]
        days: Option<String>,
        /// The alarm tone; the phone's first without
        #[arg(long, default_value = "")]
        tone: String,
    },
    /// Remove the alarm with this ID
    Remove { id: u32 },
    /// Snooze the ringing alarm
    Snooze,
    /// End the ringing or snoozed alarm
    Dismiss,
}

const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn parse_time(text: &str) -> Option<(u8, u8)> {
    let (h, m) = text.split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?))
}

fn parse_days(text: &str) -> Option<u8> {
    text.split(',').try_fold(0u8, |mask, day| {
        let i = DAYS.iter().position(|d| day.trim().to_ascii_lowercase().starts_with(d))?;
        Some(mask | 1 << i)
    })
}

fn describe(a: &Alarm) -> String {
    let days = match a.days {
        0 => "one time".to_string(),
        Alarm::EVERY_DAY => "every day".to_string(),
        mask => (0..7).filter(|i| mask & (1 << i) != 0).map(|i| DAYS[i]).collect::<Vec<_>>().join(","),
    };
    format!("{} {:02}:{:02} {} {} {}\n", a.id, a.hour, a.minute, days, a.tone, if a.enabled { "on" } else { "off" })
}

fn register_diag(ctx: &Context) {
    let store = ctx.store.clone();
    ctx.diag.register(move |args: AlarmsCommand, out: crate::diag::Output| {
        let store = store.clone();
        async move {
            let command = match args.action {
                None | Some(AlarmsAction::List) => None,
                Some(AlarmsAction::Add { time, days, tone }) => {
                    let (Some((hour, minute)), Some(days)) =
                        (parse_time(&time), days.as_deref().map_or(Some(0), parse_days))
                    else {
                        out.err("dphone alarms: the time is HH:MM, and the days are like mon,wed,fri\n").await;
                        return 2;
                    };
                    Some(Command::SetAlarm { alarm: Alarm { id: 0, hour, minute, days, tone, enabled: true } })
                }
                Some(AlarmsAction::Remove { id }) => Some(Command::RemoveAlarm { id }),
                Some(AlarmsAction::Snooze) => Some(Command::SnoozeAlarm),
                Some(AlarmsAction::Dismiss) => Some(Command::DismissAlarm),
            };
            if let Some(command) = command {
                let pending = store.send(command);
                let result = tokio::task::spawn_blocking(move || pending.wait_timeout(Duration::from_secs(5))).await;
                if let Ok(Err(e)) = result {
                    out.err(&format!("dphone alarms: {e:?}\n")).await;
                    return 1;
                }
            }
            let st = store.get::<Alarms>();
            let mut text: String = st.alarms.iter().map(describe).collect();
            if let Some(r) = &st.ringing {
                text.push_str(&format!("ringing: {}\n", r.id));
            }
            if let Some((id, until)) = st.snoozed {
                text.push_str(&format!("snoozed: {id} until {until}\n"));
            }
            if let Some((id, at)) = st.next {
                text.push_str(&format!("next: {id} at {at}\n"));
            }
            out.out(&text).await;
            0
        }
    });
}

pub fn start(ctx: &Context, audio: AudioService) -> AlarmService {
    let shared = Arc::new(Shared { zone: Mutex::new(Zone::new("")), changed: Notify::new() });
    let path = ctx.paths.nvdata.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!("alarms: cannot read {}: {e}", path.display());
            None
        }
    };
    let runner = Runner {
        ctx: ctx.clone(),
        audio,
        shared: shared.clone(),
        engine: Engine::load(text.as_deref()),
        path,
        tones: super::alarmtones::names(),
    };
    tokio::spawn(runner.run());
    register_diag(ctx);
    AlarmService { shared }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_days_of_the_diagnostic_command() {
        assert_eq!(parse_time("08:05"), Some((8, 5)));
        assert_eq!(parse_time("8"), None);
        assert_eq!(parse_days("mon,Wed,friday"), Some(0b0101010));
        assert_eq!(parse_days("sun"), Some(1));
        assert_eq!(parse_days("mon,noday"), None);
        let a = Alarm { id: 3, hour: 8, minute: 0, days: 0b0000010, tone: "Osmium".into(), enabled: true };
        assert_eq!(describe(&a), "3 08:00 mon Osmium on\n");
    }
}
