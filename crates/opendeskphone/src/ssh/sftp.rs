//! SFTP subsystem: enough of version 3 for OpenSSH's `sftp` and `scp`
//! (which speaks SFTP by default) to browse, fetch, and push files.  The
//! session runs as root with the whole filesystem visible, like the shell.

use std::collections::HashMap;
use std::io::SeekFrom;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// Open handles per session.
const MAX_HANDLES: usize = 64;
/// Largest read reply.
const MAX_READ: u32 = 256 * 1024;

enum Open {
    File(tokio::fs::File),
    Dir { path: PathBuf, done: bool },
}

pub struct Sftp {
    cwd: PathBuf,
    handles: HashMap<String, Open>,
    next: u64,
}

impl Sftp {
    pub fn new(cwd: PathBuf) -> Self {
        Sftp { cwd, handles: HashMap::new(), next: 0 }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        let joined = if p.is_absolute() { p.to_path_buf() } else { self.cwd.join(p) };
        normalize(&joined)
    }

    fn insert(&mut self, open: Open) -> Result<String, StatusCode> {
        if self.handles.len() >= MAX_HANDLES {
            return Err(StatusCode::Failure);
        }
        self.next += 1;
        let handle = self.next.to_string();
        self.handles.insert(handle.clone(), open);
        Ok(handle)
    }

    fn file(&mut self, handle: &str) -> Result<&mut tokio::fs::File, StatusCode> {
        match self.handles.get_mut(handle) {
            Some(Open::File(f)) => Ok(f),
            _ => Err(StatusCode::Failure),
        }
    }
}

/// Resolve `.` and `..` without touching the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

fn code(e: std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".to_string(), language_tag: "en-US".to_string() }
}

fn name_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

async fn apply_attrs(path: &Path, attrs: &FileAttributes) -> std::io::Result<()> {
    if let Some(size) = attrs.size {
        tokio::fs::OpenOptions::new().write(true).open(path).await?.set_len(size).await?;
    }
    if let Some(mode) = attrs.permissions {
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777)).await?;
    }
    // Times and ownership are accepted and ignored.
    Ok(())
}

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _version: u32, _extensions: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(&mut self, id: u32, filename: String, pflags: OpenFlags, attrs: FileAttributes) -> Result<Handle, Self::Error> {
        let path = self.resolve(&filename);
        let mut options = tokio::fs::OpenOptions::new();
        options
            .read(pflags.contains(OpenFlags::READ))
            .write(pflags.contains(OpenFlags::WRITE))
            .append(pflags.contains(OpenFlags::APPEND))
            .truncate(pflags.contains(OpenFlags::TRUNCATE))
            .mode(attrs.permissions.map_or(0o644, |m| m & 0o7777));
        if pflags.contains(OpenFlags::CREATE) {
            if pflags.contains(OpenFlags::EXCLUDE) {
                options.create_new(true);
            } else {
                options.create(true);
            }
        }
        let file = options.open(&path).await.map_err(code)?;
        let handle = self.insert(Open::File(file))?;
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        match self.handles.remove(&handle) {
            Some(Open::File(mut f)) => {
                f.flush().await.map_err(code)?;
                Ok(ok(id))
            }
            Some(Open::Dir { .. }) => Ok(ok(id)),
            None => Err(StatusCode::Failure),
        }
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, Self::Error> {
        let file = self.file(&handle)?;
        file.seek(SeekFrom::Start(offset)).await.map_err(code)?;
        let mut data = vec![0u8; len.min(MAX_READ) as usize];
        let mut filled = 0;
        while filled < data.len() {
            match file.read(&mut data[filled..]).await.map_err(code)? {
                0 => break,
                n => filled += n,
            }
        }
        if filled == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(filled);
        Ok(Data { id, data })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, Self::Error> {
        let file = self.file(&handle)?;
        file.seek(SeekFrom::Start(offset)).await.map_err(code)?;
        file.write_all(&data).await.map_err(code)?;
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = tokio::fs::symlink_metadata(self.resolve(&path)).await.map_err(code)?;
        Ok(Attrs { id, attrs: FileAttributes::from(&meta) })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let meta = tokio::fs::metadata(self.resolve(&path)).await.map_err(code)?;
        Ok(Attrs { id, attrs: FileAttributes::from(&meta) })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let meta = self.file(&handle)?.metadata().await.map_err(code)?;
        Ok(Attrs { id, attrs: FileAttributes::from(&meta) })
    }

    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        apply_attrs(&self.resolve(&path), &attrs).await.map_err(code)?;
        Ok(ok(id))
    }

    async fn fsetstat(&mut self, id: u32, handle: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        let file = self.file(&handle)?;
        if let Some(size) = attrs.size {
            file.set_len(size).await.map_err(code)?;
        }
        if let Some(mode) = attrs.permissions {
            file.set_permissions(std::fs::Permissions::from_mode(mode & 0o7777)).await.map_err(code)?;
        }
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let path = self.resolve(&path);
        let meta = tokio::fs::metadata(&path).await.map_err(code)?;
        if !meta.is_dir() {
            return Err(StatusCode::NoSuchFile);
        }
        let handle = self.insert(Open::Dir { path, done: false })?;
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let path = match self.handles.get_mut(&handle) {
            Some(Open::Dir { done: true, .. }) => return Err(StatusCode::Eof),
            Some(Open::Dir { path, done }) => {
                *done = true;
                path.clone()
            }
            _ => return Err(StatusCode::Failure),
        };
        let mut files = Vec::new();
        let mut entries = tokio::fs::read_dir(&path).await.map_err(code)?;
        while let Some(entry) = entries.next_entry().await.map_err(code)? {
            let attrs = match tokio::fs::symlink_metadata(entry.path()).await {
                Ok(meta) => FileAttributes::from(&meta),
                Err(_) => FileAttributes::default(),
            };
            files.push(File::new(entry.file_name().to_string_lossy().into_owned(), attrs));
        }
        Ok(Name { id, files })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        tokio::fs::remove_file(self.resolve(&filename)).await.map_err(code)?;
        Ok(ok(id))
    }

    async fn mkdir(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        let path = self.resolve(&path);
        tokio::fs::create_dir(&path).await.map_err(code)?;
        if let Some(mode) = attrs.permissions {
            let _ = tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode & 0o7777)).await;
        }
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        tokio::fs::remove_dir(self.resolve(&path)).await.map_err(code)?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let resolved = self.resolve(&path);
        let real = tokio::fs::canonicalize(&resolved).await.unwrap_or(resolved);
        Ok(Name { id, files: vec![File::dummy(name_of(&real))] })
    }

    async fn rename(&mut self, id: u32, oldpath: String, newpath: String) -> Result<Status, Self::Error> {
        tokio::fs::rename(self.resolve(&oldpath), self.resolve(&newpath)).await.map_err(code)?;
        Ok(ok(id))
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let target = tokio::fs::read_link(self.resolve(&path)).await.map_err(code)?;
        Ok(Name { id, files: vec![File::dummy(name_of(&target))] })
    }

    async fn symlink(&mut self, id: u32, linkpath: String, targetpath: String) -> Result<Status, Self::Error> {
        tokio::fs::symlink(targetpath, self.resolve(&linkpath)).await.map_err(code)?;
        Ok(ok(id))
    }
}

/// Wraps the SFTP channel stream so that when the SFTP server is done with
/// it (the client closed its side), the channel gets an exit status and is
/// closed.  Without the exit status, `scp` reports failure.
pub struct CloseOnDrop<S: Send + 'static> {
    inner: Option<S>,
    handle: Option<russh::server::Handle>,
    id: russh::ChannelId,
}

impl<S: Send + 'static> CloseOnDrop<S> {
    pub fn new(inner: S, handle: russh::server::Handle, id: russh::ChannelId) -> Self {
        CloseOnDrop { inner: Some(inner), handle: Some(handle), id }
    }
}

impl<S: Send + 'static> Drop for CloseOnDrop<S> {
    fn drop(&mut self) {
        let (Some(handle), Some(inner), id) = (self.handle.take(), self.inner.take(), self.id) else { return };
        // Dropped on the runtime (inside the SFTP task); if the runtime is
        // already gone there is nobody to tell.
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = handle.exit_status_request(id, 0).await;
                let _ = handle.eof(id).await;
                // Dropping the channel stream sends the close, after the
                // two messages above.
                drop(inner);
            });
        }
    }
}

impl<S: tokio::io::AsyncRead + Unpin + Send + 'static> tokio::io::AsyncRead for CloseOnDrop<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.inner.as_mut() {
            Some(inner) => std::pin::Pin::new(inner).poll_read(cx, buf),
            None => std::task::Poll::Ready(Ok(())),
        }
    }
}

impl<S: tokio::io::AsyncWrite + Unpin + Send + 'static> tokio::io::AsyncWrite for CloseOnDrop<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.inner.as_mut() {
            Some(inner) => std::pin::Pin::new(inner).poll_write(cx, buf),
            None => std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into())),
        }
    }

    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        match self.inner.as_mut() {
            Some(inner) => std::pin::Pin::new(inner).poll_flush(cx),
            None => std::task::Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        match self.inner.as_mut() {
            Some(inner) => std::pin::Pin::new(inner).poll_shutdown(cx),
            None => std::task::Poll::Ready(Ok(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), Path::new("/a/c/d"));
        assert_eq!(normalize(Path::new("/..")), Path::new("/"));
        let s = Sftp::new(PathBuf::from("/nvdata/root"));
        assert_eq!(s.resolve("x/y"), Path::new("/nvdata/root/x/y"));
        assert_eq!(s.resolve("/tmp/opendeskphone"), Path::new("/tmp/opendeskphone"));
        assert_eq!(s.resolve("."), Path::new("/nvdata/root"));
    }
}
