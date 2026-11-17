//! The command line of the multi-call binary, defined with clap.
//!
//! | Invocation | Role |
//! | --- | --- |
//! | PID 1 (as `/sbin/init`) | init role |
//! | `dphone main [options]` | main role |
//! | `dphone mount ...`, or a symlink named `mount`, etc. | system command |
//! | `dphone update ...`, `dphone ramboot ...`, `dphone supervise ...` | run in the calling process |
//! | `dphone <anything else>` | diagnostic command sent to the main role |
//!
//! There is one command tree, [`Invocation`].  The commands that run in
//! the calling process ([`Command`]'s own variants and [`SystemCommand`])
//! are defined beside the code that runs them.  So are the commands the
//! running main role answers, each a type its subsystem registers a
//! handler for (see [`crate::diag::Registry`]); [`Remote`] lists them, which
//! is what puts them in the tree.
//!
//! The process that is called parses the whole command line, so `dphone
//! help`, `dphone COMMAND --help`, and the report of a mistake are clap's,
//! made from that one tree, and need no main role.  For a command of
//! [`Remote`] it then sends the arguments, as they were given, to the main
//! role, which parses them again with the same definition into the type
//! the handler takes.  (Sending the arguments, not the parsed value, keeps
//! the socket's protocol a list of words: there is nothing to serialize,
//! and whatever else speaks it, e.g., a test, is checked the same way.
//! Both ends are this one binary, so the two parses cannot disagree.)

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::cmds::SystemCommand;
use crate::diag::builtin::{HealthCommand, LogCommand, PingCommand, RestartCommand, VersionCommand};
use crate::init;
use crate::mainrole::subsystems::{
    ConfigCommand, StateCommand,
    alarms::AlarmsCommand,
    audio::{AudioCommand, DspCommand},
    ehs::EhsCommand,
    expansion::ExpansionCommand,
    hal::HalCommand,
    liveness::CrashlogCommand,
    net::NetCommand,
    paging::PageCommand,
    remote::{AppCommand, CallCommand, HookCommand, KeyCommand, ScreenshotCommand},
};

/// What the binary was called as: `dphone`, or the name of one of the
/// system commands it provides (the image has a symbolic link for each).
#[derive(Debug, Parser)]
#[command(multicall = true)]
enum Invocation {
    /// Open DPhone: the phone's one program
    #[command(version = crate::VERSION, after_help = where_commands_run())]
    Dphone {
        #[command(subcommand)]
        command: Command,
    },
    #[command(flatten)]
    System(SystemCommand),
    /// Any other name (e.g., `/sbin/init` when it is not PID 1, or a copy
    /// under test): `dphone`.
    #[command(external_subcommand)]
    Other(Vec<OsString>),
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the main role
    Main(crate::mainrole::Options),
    /// Run the init role's supervisor without the duties of PID 1 (for tests)
    ///
    /// The init role proper runs only as PID 1.
    Supervise(Supervise),
    /// Firmware: show the banks, install an image, select a bank
    Update {
        #[command(subcommand)]
        command: crate::update_cli::UpdateCommand,
    },
    /// Start an image from RAM without installing it
    Ramboot(crate::ramboot_cli::Ramboot),
    /// Print the shell's tab completion of `dphone`
    ///
    /// A Bash script, which brush reads, made from this command tree.  The
    /// phone's `/etc/bash.bashrc` loads it the first time a `dphone`
    /// command line is completed.
    Completion,
    #[command(flatten)]
    System(SystemCommand),
    #[command(flatten)]
    Remote(Remote),
}

/// The commands the running main role answers.  A subsystem adds its
/// command here and registers the handler when it starts.
#[derive(Debug, Subcommand)]
pub enum Remote {
    Alarms(AlarmsCommand),
    App(AppCommand),
    Audio(AudioCommand),
    Call(CallCommand),
    Config(ConfigCommand),
    Crashlog(CrashlogCommand),
    Dsp(DspCommand),
    Ehs(EhsCommand),
    Expansion(ExpansionCommand),
    Hal(HalCommand),
    Health(HealthCommand),
    Hook(HookCommand),
    Key(KeyCommand),
    Log(LogCommand),
    Net(NetCommand),
    Page(PageCommand),
    Ping(PingCommand),
    Restart(RestartCommand),
    Screenshot(ScreenshotCommand),
    State(StateCommand),
    Version(VersionCommand),
}

/// Which commands need the main role, for the end of `dphone --help`.
fn where_commands_run() -> String {
    let names = |cmd: clap::Command| cmd.get_subcommands().map(|c| c.get_name().to_string()).collect::<Vec<_>>();
    let remote = names(Remote::augment_subcommands(clap::Command::new("dphone")));
    let mut local = names(Command::augment_subcommands(clap::Command::new("dphone")));
    local.retain(|name| !remote.contains(name));
    format!(
        "These run in the calling process, with or without a main role:\n  {}\n\
         The other commands are answered by the running main role.",
        local.join(" ")
    )
}

/// `on` or `off`, for the commands that switch something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl OnOff {
    pub fn is_on(self) -> bool {
        self == OnOff::On
    }
}

#[derive(Debug, Args)]
struct Supervise {
    /// The nvdata directory
    #[arg(long, value_name = "DIR")]
    nvdata: PathBuf,
    /// The runtime directory
    #[arg(long, value_name = "DIR")]
    run_dir: PathBuf,
    /// How long the program has to report itself healthy after a start
    #[arg(long, value_name = "SECONDS", default_value_t = init::START_GRACE_SECS)]
    grace: u64,
    /// How long the program may go without a heartbeat
    #[arg(long, value_name = "SECONDS", default_value_t = init::BEAT_TIMEOUT_SECS)]
    beat_timeout: u64,
    /// How long the program may stay unhealthy before the phone would reboot
    #[arg(long, value_name = "SECONDS", default_value_t = init::UNHEALTHY_REBOOT_SECS)]
    unhealthy_reboot: u64,
    /// A file to write in place of a return to the other bank
    #[arg(long, value_name = "FILE")]
    rollback_marker: Option<PathBuf>,
    /// The program to supervise as the main role, and its arguments
    #[arg(last = true, required = true, value_name = "PROGRAM")]
    program: Vec<String>,
}

/// Hooks other crates supply to the init role.  The updater's rollback
/// function is added here when that crate is wired in.
fn init_hooks() -> init::Hooks {
    init::Hooks { rollback: Some(crate::update_cli::rollback_hook) }
}

/// The whole command tree, e.g., for a front end that shows help.
pub fn command() -> clap::Command {
    Invocation::command()
}

/// `dphone completion`: the Bash completion of `dphone` and every command
/// under it, by clap_complete.  The root of the tree is the multicall
/// root, which nobody runs, so the script is made from `dphone`'s part.
fn completion_script() -> Vec<u8> {
    let mut dphone = command().find_subcommand("dphone").expect("'dphone' is a name of the tree").clone();
    let mut script = Vec::new();
    clap_complete::generate(clap_complete::shells::Bash, &mut dphone, "dphone", &mut script);
    script
}

pub fn run(args: Vec<OsString>) -> i32 {
    // PID 1 is always the init role, whatever the name or arguments.
    if crate::sys::getpid() == 1 {
        init::run(init_hooks());
    }
    let invocation = match Invocation::try_parse_from(&args) {
        Ok(Invocation::Other(mut words)) => {
            words[0] = "dphone".into();
            Invocation::try_parse_from(&words)
        }
        other => other,
    };
    match invocation {
        Ok(Invocation::Dphone { command }) => match command {
            Command::Main(options) => crate::mainrole::run(options),
            Command::Supervise(s) => init::run_supervisor_test(init::TestOptions {
                paths: crate::paths::Paths::new(s.nvdata, s.run_dir),
                main_command: s.program,
                start_grace: s.grace,
                beat_timeout: s.beat_timeout,
                unhealthy_reboot: s.unhealthy_reboot,
                rollback_marker: s.rollback_marker,
            }),
            Command::Update { command } => crate::update_cli::run(command),
            Command::Ramboot(ramboot) => crate::ramboot_cli::run(&ramboot),
            Command::Completion => {
                crate::out::stdout_bytes(&completion_script());
                0
            }
            Command::System(command) => crate::cmds::run(command),
            // Checked here; parsed again, into the handler's type, there.
            Command::Remote(_) => {
                let words: Vec<String> = args.iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
                crate::diag::client::run(&crate::paths::client_diag_socket(), &words)
            }
        },
        Ok(Invocation::System(command)) => crate::cmds::run(command),
        Ok(Invocation::Other(_)) => unreachable!("'dphone' is a name of the tree"),
        Err(e) => report(&e),
    }
}

/// Print help that was asked for, or what is wrong with the command line;
/// returns the exit status (0 and 2, as clap has them).
fn report(e: &clap::Error) -> i32 {
    let text = e.render().to_string();
    if e.use_stderr() {
        crate::err!("{text}");
        2
    } else {
        crate::out!("{text}");
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse(words: &[&str]) -> Result<Invocation, clap::Error> {
        Invocation::try_parse_from(words)
    }

    fn kind(words: &[&str]) -> ErrorKind {
        parse(words).expect_err(&words.join(" ")).kind()
    }

    /// Every command of the tree, with the words that reach it.
    fn every_command(cmd: &clap::Command, path: &mut Vec<String>, found: &mut Vec<Vec<String>>) {
        found.push(path.clone());
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            path.push(sub.get_name().to_string());
            every_command(sub, path, found);
            path.pop();
        }
    }

    fn all_paths() -> Vec<Vec<String>> {
        let mut found = Vec::new();
        every_command(&command(), &mut Vec::new(), &mut found);
        // The multicall root itself is not a command anyone can run.
        found.remove(0);
        found
    }

    #[test]
    fn the_tree_is_well_formed() {
        command().debug_assert();
    }

    #[test]
    fn every_command_renders_help_and_has_a_description() {
        let paths = all_paths();
        // Local commands, system commands by name and under `dphone`, the
        // main role's commands, and their subcommands.
        assert!(paths.len() > 80, "{}", paths.len());
        for path in paths {
            let mut words = path.clone();
            words.push("--help".into());
            let e = Invocation::try_parse_from(&words).expect_err("help is not a parse");
            assert_eq!(e.kind(), ErrorKind::DisplayHelp, "{words:?}");
            assert!(!e.use_stderr());
            let text = e.render().to_string();
            assert!(text.contains(&format!("Usage: {}", path.join(" "))), "{words:?}: {text}");
            let mut cmd = command();
            let mut at = &mut cmd;
            for word in &path {
                at = at.find_subcommand_mut(word).unwrap();
            }
            assert!(at.get_about().is_some(), "{path:?} has no description");
        }
    }

    /// Each of the main role's commands is in the tree under the name its
    /// own definition has, which is the name its handler is registered and
    /// looked up by, and is well formed alone, as the main role parses it.
    #[test]
    fn the_main_roles_commands_are_in_the_tree_under_their_own_names() {
        let tree = Remote::augment_subcommands(clap::Command::new("dphone"));
        let in_tree: Vec<&str> = tree.get_subcommands().map(|c| c.get_name()).collect();
        let own = [
            AlarmsCommand::command(),
            AppCommand::command(),
            AudioCommand::command(),
            CallCommand::command(),
            ConfigCommand::command(),
            CrashlogCommand::command(),
            DspCommand::command(),
            EhsCommand::command(),
            ExpansionCommand::command(),
            HalCommand::command(),
            HealthCommand::command(),
            HookCommand::command(),
            KeyCommand::command(),
            LogCommand::command(),
            NetCommand::command(),
            PageCommand::command(),
            PingCommand::command(),
            RestartCommand::command(),
            ScreenshotCommand::command(),
            StateCommand::command(),
            VersionCommand::command(),
        ];
        let own_names: Vec<&str> = own.iter().map(|c| c.get_name()).collect();
        assert_eq!(in_tree, own_names);
        for command in own {
            command.debug_assert();
        }
    }

    #[test]
    fn dphone_and_its_other_names() {
        assert!(matches!(parse(&["dphone", "ping"]), Ok(Invocation::Dphone { command: Command::Remote(Remote::Ping(_)) })));
        // By path, as the shell runs it.
        assert!(matches!(parse(&["/bin/dphone", "ping"]), Ok(Invocation::Dphone { .. })));
        // A system command by its own name, and as a command of `dphone`.
        assert!(matches!(parse(&["/bin/lsmod"]), Ok(Invocation::System(SystemCommand::Lsmod))));
        assert!(matches!(
            parse(&["dphone", "lsmod"]),
            Ok(Invocation::Dphone { command: Command::System(SystemCommand::Lsmod) })
        ));
        // Any other name is left for `dphone`.
        let Ok(Invocation::Other(words)) = parse(&["/sbin/init", "ping"]) else { panic!("not left over") };
        assert_eq!(words, ["init", "ping"]);
        // A system command's help names it as it was called.
        let help = parse(&["umount", "--help"]).unwrap_err().render().to_string();
        assert!(help.contains("Usage: umount [OPTIONS] <TARGET>..."), "{help}");
        let help = parse(&["dphone", "umount", "--help"]).unwrap_err().render().to_string();
        assert!(help.contains("Usage: dphone umount [OPTIONS] <TARGET>..."), "{help}");
    }

    #[test]
    fn help_version_and_mistakes() {
        assert_eq!(kind(&["dphone", "--help"]), ErrorKind::DisplayHelp);
        assert_eq!(kind(&["dphone", "help"]), ErrorKind::DisplayHelp);
        assert_eq!(kind(&["dphone", "help", "update"]), ErrorKind::DisplayHelp);
        assert_eq!(kind(&["dphone", "--version"]), ErrorKind::DisplayVersion);
        let e = parse(&["dphone", "--version"]).unwrap_err();
        assert_eq!(e.render().to_string(), format!("dphone {}\n", crate::VERSION));
        // Nothing to do is a mistake, answered with the help.
        let e = parse(&["dphone"]).unwrap_err();
        assert!(e.use_stderr());
        assert!(e.render().to_string().contains("Usage: dphone <COMMAND>"));
        assert_eq!(kind(&["dphone", "no-such-command"]), ErrorKind::InvalidSubcommand);
        assert_eq!(kind(&["dphone", "ping", "extra"]), ErrorKind::UnknownArgument);
        // The top-level help lists commands of every kind, and says which
        // need the main role.
        let help = parse(&["dphone", "--help"]).unwrap_err().render().to_string();
        // (Help is wrapped to the width of the terminal.)
        let words = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            words.contains(
                "with or without a main role: main supervise update ramboot completion mount umount insmod rmmod lsmod modprobe \
                 reboot poweroff halt dmesg ps The other commands"
            ),
            "{help}"
        );
        // A near miss gets a suggestion.
        let e = parse(&["dphone", "screenshoot"]).unwrap_err().render().to_string();
        assert!(e.contains("a similar subcommand exists: 'screenshot'"), "{e}");
        for name in ["main", "update", "ramboot", "mount", "ps", "log", "state", "config", "hal", "health", "key"] {
            assert!(help.lines().any(|l| l.trim_start().starts_with(&format!("{name} "))), "help lacks {name}: {help}");
        }
    }

    #[test]
    fn supervise() {
        let Ok(Invocation::Dphone { command: Command::Supervise(s) }) = parse(&[
            "dphone",
            "supervise",
            "--nvdata",
            "/nv",
            "--run-dir",
            "/run",
            "--beat-timeout",
            "3",
            "--",
            "dphone",
            "main",
            "--sim",
        ]) else {
            panic!("supervise does not parse")
        };
        assert_eq!(s.program, ["dphone", "main", "--sim"]);
        assert_eq!((s.grace, s.beat_timeout), (init::START_GRACE_SECS, 3));
        assert_eq!(kind(&["dphone", "supervise", "--nvdata", "/nv", "--run-dir", "/run"]), ErrorKind::MissingRequiredArgument);
        assert_eq!(kind(&["dphone", "supervise", "--run-dir", "/run", "--", "x"]), ErrorKind::MissingRequiredArgument);
    }

    /// The main role's commands whose arguments are more than a word.
    #[test]
    fn the_main_roles_commands() {
        let ok = |words: &[&str]| {
            let mut all = vec!["dphone"];
            all.extend(words);
            assert!(matches!(parse(&all), Ok(Invocation::Dphone { command: Command::Remote(_) })), "{words:?}");
        };
        let bad = |words: &[&str]| {
            let mut all = vec!["dphone"];
            all.extend(words);
            assert!(parse(&all).is_err(), "{words:?}");
        };
        // Keys: one or more, each a key of the phone.
        ok(&["key", "1", "2", "3", "sk1", "#", "*"]);
        ok(&["key", "line6", "vup"]);
        bad(&["key"]);
        bad(&["key", "1", "nope"]);
        assert_eq!(kind(&["dphone", "key", "list"]), ErrorKind::ValueValidation);
        let help = parse(&["dphone", "key", "--help"]).unwrap_err().render().to_string();
        assert!(help.contains("sk1..sk4") && help.contains("blfleft"), "{help}");

        ok(&["hook", "off"]);
        bad(&["hook", "up"]);
        bad(&["hook"]);

        // A bare noun is its status or list.
        for noun in ["app", "audio", "call", "dsp", "hal", "net", "page", "state"] {
            ok(&[noun]);
        }
        bad(&["config"]);
        ok(&["config", "set", "open_x", "a value"]);
        bad(&["config", "set", "open_x"]);
        ok(&["config", "set", "open_x", "-5"]);
        ok(&["config", "list"]);
        ok(&["config", "list", "sip"]);

        ok(&["log"]);
        ok(&["log", "-n", "50", "-f"]);
        ok(&["log", "--lines", "50", "--follow"]);
        bad(&["log", "-n", "many"]);

        ok(&["net", "bridge", "on"]);
        ok(&["net", "bridge", "off", "--save"]);
        bad(&["net", "bridge"]);
        bad(&["net", "bridge", "maybe"]);

        ok(&["audio", "volume", "ringer", "10"]);
        bad(&["audio", "volume", "ringer", "11"]);
        bad(&["audio", "volume", "bell", "3"]);
        ok(&["audio", "ring"]);
        ok(&["audio", "ring", "off"]);
        ok(&["audio", "tone", "ringback"]);
        bad(&["audio", "tone", "chirp"]);

        ok(&["dsp", "job", "1", "2", "3", "4", "5", "6", "16"]);
        ok(&["dsp", "job", "1", "2", "3", "4", "5", "6", "16", "43690"]);
        bad(&["dsp", "job", "1", "2", "3", "4", "5", "6"]);
        bad(&["dsp", "job", "1", "2", "3", "4", "5", "6", "70000"]);
        ok(&["dsp", "bands", "0", "--sweep"]);
        ok(&["dsp", "bands", "3", "--tone", "1000", "--frames", "100"]);
        bad(&["dsp", "bands", "0"]);
        bad(&["dsp", "bands", "0", "--sweep", "--tone", "1000"]);
        bad(&["dsp", "bands", "4", "--sweep"]);
        bad(&["dsp", "bands", "0", "--sweep", "--frames", "4"]);
        ok(&["dsp", "canceller", "sbaec", "ingress", "1", "1000"]);
        bad(&["dsp", "canceller", "hec2", "ingress", "1", "1000"]);

        ok(&["hal", "lamp", "line3", "green"]);
        ok(&["hal", "lamp", "all", "off"]);
        bad(&["hal", "lamp", "line0", "green"]);
        bad(&["hal", "lamp", "line3", "blue"]);
        ok(&["hal", "pattern"]);
        ok(&["hal", "pattern", "white"]);
        bad(&["hal", "backlight", "300"]);

        ok(&["page", "start", "dock"]);
        bad(&["page", "start"]);
        ok(&["screenshot", "/tmp/a.ppm"]);
        ok(&["screenshot", "--screen", "side", "/tmp/a.pgm"]);
        bad(&["screenshot"]);
    }
}
