//! Server side of the diagnostics socket, in the main role.

use std::collections::BTreeMap;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use clap::Parser;
use log::{debug, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use super::{EXIT_UNAVAILABLE, EXIT_USAGE, FRAME_EXIT, FRAME_REQUEST, FRAME_STDERR, FRAME_STDOUT, MAX_FRAME, encode_frame};

type BoxFuture = Pin<Box<dyn Future<Output = i32> + Send>>;
type Handler = Arc<dyn Fn(Vec<String>, Output) -> BoxFuture + Send + Sync>;

/// The set of diagnostic commands the main role answers.  Cheap to clone;
/// subsystems keep a clone and register commands whenever they start.
///
/// A command is a type that derives [`clap::Parser`] (its name is the
/// command's name, e.g., `#[command(name = "hook")]`), defined beside the
/// code that answers it and listed in [`crate::cli::Remote`], which puts it
/// in the tree the `dphone` client parses with.  The client forwards the
/// arguments it has checked; they are parsed again here into the type the
/// handler takes, so a front end that calls [`Registry::dispatch`] itself
/// gets the same checks, help, and messages.
#[derive(Clone, Default)]
pub struct Registry {
    commands: Arc<Mutex<BTreeMap<String, Handler>>>,
}

/// The name of the command `T` defines.
fn name_of<T: Parser>() -> String {
    T::command().get_name().to_string()
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// Register the command `T`.  The handler gets the parsed arguments and
    /// an [`Output`]; its result is the client's exit status.  Registering a
    /// command again replaces the earlier handler, which is how a subsystem
    /// takes over a placeholder.
    pub fn register<T, F, Fut>(&self, handler: F)
    where
        T: Parser + Send + 'static,
        F: Fn(T, Output) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = i32> + Send + 'static,
    {
        let name = name_of::<T>();
        let handler = Arc::new(handler);
        let bin_name = format!("dphone {name}");
        self.insert(
            name,
            Arc::new(move |args, out| {
                let parsed = T::command()
                    .bin_name(&bin_name)
                    .no_binary_name(true)
                    .try_get_matches_from(args)
                    .and_then(|matches| T::from_arg_matches(&matches));
                let handler = handler.clone();
                Box::pin(async move {
                    match parsed {
                        Ok(args) => handler(args, out).await,
                        // Help that was asked for, or what is wrong with the
                        // arguments, in clap's words.
                        Err(e) if e.use_stderr() => {
                            out.err(&e.render().to_string()).await;
                            EXIT_USAGE
                        }
                        Err(e) => {
                            out.out(&e.render().to_string()).await;
                            0
                        }
                    }
                })
            }),
        );
    }

    /// Register the command `T` as one that cannot be answered, and why
    /// (e.g., its subsystem did not start).
    pub fn register_unavailable<T: Parser>(&self, why: &'static str) {
        let name = name_of::<T>();
        let text = format!("dphone {name}: {why}\n");
        self.insert(
            name,
            Arc::new(move |_args, out| {
                let text = text.clone();
                Box::pin(async move {
                    out.err(&text).await;
                    EXIT_UNAVAILABLE
                })
            }),
        );
    }

    fn insert(&self, name: String, handler: Handler) {
        if let Ok(mut commands) = self.commands.lock() {
            commands.insert(name, handler);
        }
    }

    /// The names of the registered commands, sorted.
    pub fn names(&self) -> Vec<String> {
        self.commands.lock().map(|c| c.keys().cloned().collect()).unwrap_or_default()
    }

    fn handler(&self, name: &str) -> Option<Handler> {
        self.commands.lock().ok()?.get(name).cloned()
    }

    /// Run a command as the socket would, for use from other front ends.
    /// `args` is the command's name and its arguments.
    pub async fn dispatch(&self, mut args: Vec<String>, out: Output) -> i32 {
        if args.is_empty() {
            return EXIT_USAGE;
        }
        let name = args.remove(0);
        match self.handler(&name) {
            Some(handler) => handler(args, out).await,
            // The client only sends commands of its tree, so this is one
            // whose subsystem has not registered it: not started (yet), or
            // not part of this phone.
            None => {
                out.err(&format!("dphone {name}: not available: nothing in the running main role answers it\n")).await;
                EXIT_UNAVAILABLE
            }
        }
    }
}

/// Where a command writes.  Sends fail quietly once the client is gone;
/// long-running commands should watch [`Output::closed`].
#[derive(Clone)]
pub struct Output {
    tx: mpsc::Sender<Vec<u8>>,
}

impl Output {
    /// An output that collects frames into a channel (also used by tests).
    pub fn channel(buffer: usize) -> (Output, mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Output { tx }, rx)
    }

    pub async fn out(&self, text: &str) {
        self.bytes(FRAME_STDOUT, text.as_bytes()).await;
    }

    pub async fn err(&self, text: &str) {
        self.bytes(FRAME_STDERR, text.as_bytes()).await;
    }

    pub async fn bytes(&self, kind: u8, data: &[u8]) {
        for chunk in data.chunks(MAX_FRAME) {
            if self.tx.send(encode_frame(kind, chunk)).await.is_err() {
                return;
            }
        }
    }

    /// Resolves when the client has gone away.
    pub async fn closed(&self) {
        self.tx.closed().await
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// Bind the socket (mode 0600) and serve until the task is dropped.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, registry: Registry) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let registry = registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, registry).await {
                        debug!("diag client: {e}");
                    }
                });
            }
            Err(e) => {
                warn!("diag accept: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

async fn handle(stream: UnixStream, registry: Registry) -> std::io::Result<()> {
    let (mut rd, mut wr) = stream.into_split();
    let mut head = [0u8; 5];
    rd.read_exact(&mut head).await?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if head[0] != FRAME_REQUEST || len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad request frame"));
    }
    let mut payload = vec![0u8; len];
    rd.read_exact(&mut payload).await?;
    let args = super::decode_request(&payload)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad request"))?;

    let (out, mut rx) = Output::channel(16);
    let command = tokio::spawn({
        let registry = registry.clone();
        async move { registry.dispatch(args, out).await }
    });
    tokio::pin!(command);

    // Relay output until the command finishes; stop early if the client
    // closes its end (read returns 0 or an error).
    let mut probe = [0u8; 1];
    let mut command_done: Option<i32> = None;
    let status = loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(frame) => {
                    if wr.write_all(&frame).await.is_err() {
                        break None;
                    }
                }
                // All Output clones dropped: the command is finished or
                // about to be; take its status.
                None => break Some(match command_done.take() {
                    Some(s) => s,
                    None => (&mut command).await.unwrap_or(1),
                }),
            },
            status = &mut command, if command_done.is_none() => {
                command_done = Some(status.unwrap_or(1));
            }
            _ = rd.read(&mut probe) => break None,
        }
    };
    match status {
        Some(status) => {
            wr.write_all(&encode_frame(FRAME_EXIT, &(status as u32).to_be_bytes())).await?;
            wr.shutdown().await?;
        }
        None => {
            // Client gone: dropping rx makes Output::closed() fire; abort in
            // case the command is not watching.
            drop(rx);
            command.abort();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Say something
    #[derive(Debug, Parser)]
    #[command(name = "say")]
    struct SayCommand {
        /// How many times
        #[arg(long, default_value_t = 1)]
        times: u8,
        word: String,
    }

    /// Never there
    #[derive(Debug, Parser)]
    #[command(name = "absent")]
    struct AbsentCommand {}

    /// Dispatch `words`; returns the exit status, standard output, and
    /// standard error.
    async fn dispatch(registry: &Registry, words: &[&str]) -> (i32, String, String) {
        let (out, mut rx) = Output::channel(64);
        let status = registry.dispatch(words.iter().map(|w| w.to_string()).collect(), out).await;
        let (mut stdout, mut stderr) = (String::new(), String::new());
        while let Some(frame) = rx.recv().await {
            let text = String::from_utf8_lossy(&frame[5..]).into_owned();
            match frame[0] {
                FRAME_STDOUT => stdout.push_str(&text),
                _ => stderr.push_str(&text),
            }
        }
        (status, stdout, stderr)
    }

    #[tokio::test]
    async fn a_command_gets_its_arguments_parsed() {
        let registry = Registry::new();
        registry.register(|args: SayCommand, out: Output| async move {
            out.out(&args.word.repeat(args.times as usize)).await;
            7
        });
        registry.register_unavailable::<AbsentCommand>("its subsystem did not start");
        assert_eq!(registry.names(), ["absent", "say"]);

        assert_eq!(dispatch(&registry, &["say", "hey"]).await, (7, "hey".into(), String::new()));
        assert_eq!(dispatch(&registry, &["say", "--times", "3", "ho"]).await, (7, "hohoho".into(), String::new()));

        // Mistakes and help are clap's, under the name the client shows.
        let (status, stdout, stderr) = dispatch(&registry, &["say"]).await;
        assert_eq!((status, stdout.as_str()), (EXIT_USAGE, ""));
        assert!(stderr.contains("Usage: dphone say <WORD>"), "{stderr}");
        let (status, _, stderr) = dispatch(&registry, &["say", "--times", "many", "x"]).await;
        assert_eq!(status, EXIT_USAGE);
        assert!(stderr.contains("invalid value 'many'"), "{stderr}");
        let (status, stdout, stderr) = dispatch(&registry, &["say", "--help"]).await;
        assert_eq!((status, stderr.as_str()), (0, ""));
        assert!(stdout.starts_with("Say something\n") && stdout.contains("Usage: dphone say [OPTIONS] <WORD>"), "{stdout}");

        // A command nothing answers, and why when that is known.
        assert_eq!(
            dispatch(&registry, &["absent"]).await,
            (EXIT_UNAVAILABLE, String::new(), "dphone absent: its subsystem did not start\n".into())
        );
        let (status, _, stderr) = dispatch(&registry, &["later"]).await;
        assert_eq!(status, EXIT_UNAVAILABLE);
        assert!(stderr.starts_with("dphone later: not available"), "{stderr}");
        assert_eq!(dispatch(&registry, &[]).await.0, EXIT_USAGE);

        // A subsystem takes a placeholder over.
        registry.register(|_: AbsentCommand, _out: Output| async { 0 });
        assert_eq!(dispatch(&registry, &["absent"]).await.0, 0);
    }
}
