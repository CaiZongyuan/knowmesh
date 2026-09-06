use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult, ErrorType};

pub(super) struct CacheWriteLease {
    _file: File,
}

impl CacheWriteLease {
    pub(super) fn acquire(path: &Path) -> AppResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(write_error)?;
        let started = Instant::now();
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self { _file: file }),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    if started.elapsed() >= Duration::from_secs(5) {
                        return Err(AppError::new(
                            ErrorType::Conflict,
                            "CACHE_BUSY",
                            "Another cache publisher is active.",
                        )
                        .retryable(true));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(write_error(error)),
            }
        }
    }
}

pub(super) fn write_artifact<T: Serialize>(
    directory: &Path,
    value: &T,
    limit: u64,
) -> AppResult<(tempfile::NamedTempFile, String, u64)> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(write_error)?;
    let mut writer = HashWriter {
        inner: temporary.as_file_mut(),
        digest: Sha256::new(),
        bytes: 0,
        limit,
    };
    if let Err(error) = serde_json::to_writer(&mut writer, value) {
        if error.io_error_kind() == Some(io::ErrorKind::FileTooLarge) {
            return Err(too_large());
        }
        return Err(AppError::new(
            ErrorType::Io,
            "CACHE_WRITE_FAILED",
            "Could not serialize the cache artifact.",
        ));
    }
    writer.flush().map_err(write_error)?;
    let bytes = writer.bytes;
    let hash = format!("{:x}", writer.digest.finalize());
    temporary.as_file().sync_all().map_err(write_error)?;
    Ok((temporary, hash, bytes))
}

pub(super) fn publish(temporary: tempfile::NamedTempFile, path: &Path) -> AppResult<()> {
    temporary
        .persist(path)
        .map_err(|error| write_error(error.error))?;
    sync_directory(
        path.parent()
            .ok_or_else(|| write_error(io::Error::other("missing cache parent")))?,
    )
}

pub(super) fn read_artifact<T: DeserializeOwned>(
    path: &Path,
    expected: Option<(&str, u64)>,
    limit: u64,
) -> AppResult<Option<T>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(read_error(error)),
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(read_error(error)),
    };
    let size = file.metadata().map_err(read_error)?.len();
    if size > limit || expected.is_some_and(|(_, bytes)| bytes != size) {
        return Ok(None);
    }
    let mut reader = HashReader {
        inner: file.take(limit.saturating_add(1)),
        digest: Sha256::new(),
        bytes: 0,
    };
    let value = match serde_json::from_reader::<_, T>(&mut reader) {
        Ok(value) => value,
        Err(error) if error.is_io() => {
            return Err(AppError::new(
                ErrorType::Io,
                "CACHE_READ_FAILED",
                "Could not read the cache artifact.",
            ));
        }
        Err(_) => return Ok(None),
    };
    if reader.bytes > limit
        || expected.is_some_and(|(hash, bytes)| {
            bytes != reader.bytes || hash != format!("{:x}", reader.digest.finalize())
        })
    {
        return Ok(None);
    }
    Ok(Some(value))
}

pub(super) fn ensure_directory(path: &Path) -> AppResult<()> {
    if path.is_dir() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| write_error(io::Error::other("missing cache parent")))?;
    ensure_directory(parent)?;
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => {}
        Err(error) => return Err(write_error(error)),
    }
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> AppResult<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(write_error)
}
#[cfg(not(unix))]
fn sync_directory(_: &Path) -> AppResult<()> {
    Ok(())
}

struct HashWriter<W> {
    inner: W,
    digest: Sha256,
    bytes: u64,
    limit: u64,
}
impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.bytes.saturating_add(buffer.len() as u64) > self.limit {
            return Err(io::Error::new(io::ErrorKind::FileTooLarge, "cache limit"));
        }
        let written = self.inner.write(buffer)?;
        self.digest.update(&buffer[..written]);
        self.bytes += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct HashReader<R> {
    inner: R,
    digest: Sha256,
    bytes: u64,
}
impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.digest.update(&buffer[..read]);
        self.bytes += read as u64;
        Ok(read)
    }
}

fn too_large() -> AppError {
    AppError::new(
        ErrorType::Policy,
        "CACHE_ARTIFACT_TOO_LARGE",
        "The cache artifact exceeds its configured size limit.",
    )
}
fn write_error(error: io::Error) -> AppError {
    AppError::new(
        ErrorType::Io,
        "CACHE_WRITE_FAILED",
        "Could not durably publish the cache artifact.",
    ).with_details(serde_json::json!({"io_kind": format!("{:?}", error.kind()), "os_code": error.raw_os_error()}))
}
fn read_error(_: io::Error) -> AppError {
    AppError::new(
        ErrorType::Io,
        "CACHE_READ_FAILED",
        "Could not read the cache artifact.",
    )
}
