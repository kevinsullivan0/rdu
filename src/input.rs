//! Bounded explicit data-file reads, without waiting for FIFO writers or following final links.
use std::{
    fs::OpenOptions,
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
pub fn read_regular(path: &Path, limit: u64, label: &str, size_label: &str) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} input must be a regular file"),
        ));
    }
    let oversized = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} exceeds {size_label}"),
        )
    };
    if metadata.len() > limit {
        return Err(oversized());
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    let mut reader = file.take(limit.saturating_add(1));
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(contents);
        }
        if contents.len() as u64 + count as u64 > limit {
            return Err(oversized());
        }
        contents.extend_from_slice(&buffer[..count]);
    }
}
