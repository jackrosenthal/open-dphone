//! Server side of the diagnostics socket, in the main role.

use std::collections::BTreeMap;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use log::{debug, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use super::{EXIT_UNKNOWN, FRAME_EXIT, FRAME_REQUEST, FRAME_STDERR, FRAME_STDOUT, MAX_FRAME, encode_frame};

type BoxFuture = Pin<Box<dyn Future<Output = i32> + Send>>;
type Handler = Arc<dyn Fn(Vec<String>, Output) -> BoxFuture + Send + Sync>;

struct Command {
    help: String,
    handler: Handler,
}

/// The set of diagnostic commands.  Cheap to clone; subsystems keep a clone
/// and register commands whenever they start.
#[derive(Clone, Default)]
pub struct Registry {
    commands: Arc<Mutex<BTreeMap<String, Command>>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// Register `name`.  The handler gets the arguments after the command
    /// name and an [`Output`]; its result is the client's exit status.
    /// Registering a name again replaces the earlier handler, which is how a
    /// subsystem takes over a placeholder.
    pub fn register<F, Fut>(&self, name: &str, help: &str, handler: F)
    where
        F: Fn(Vec<String>, Output) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = i32> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |args, out| Box::pin(handler(args, out)));
        if let Ok(mut commands) = self.commands.lock() {
            commands.insert(name.to_string(), Command { help: help.to_string(), handler });
        }
    }

    /// (name, help) for every command, sorted by name.
    pub fn list(&self) -> Vec<(String, String)> {
        match self.commands.lock() {
            Ok(commands) => commands.iter().map(|(n, c)| (n.clone(), c.help.clone())).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn handler(&self, name: &str) -> Option<Handler> {
        self.commands.lock().ok()?.get(name).map(|c| c.handler.clone())
    }

    /// Run a command as the socket would, for use from other front ends.
    pub async fn dispatch(&self, mut args: Vec<String>, out: Output) -> i32 {
        if args.is_empty() {
            return EXIT_UNKNOWN;
        }
        let name = args.remove(0);
        match self.handler(&name) {
            Some(handler) => handler(args, out).await,
            None => {
                out.err(&format!("dphone: unknown command '{name}' (try 'dphone help')\n")).await;
                EXIT_UNKNOWN
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
