//! Small audited FFI boundary. File contents are never opened.
use crate::scan::Kind;
use std::{
    ffi::{CStr, CString, OsString},
    io,
    os::{
        fd::{FromRawFd, OwnedFd, RawFd},
        unix::ffi::OsStringExt,
    },
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Meta {
    pub kind: Kind,
    pub dev: u64,
    pub ino: u64,
    pub links: u64,
    pub allocated: u64,
    pub apparent: u64,
}
pub(crate) struct Raw {
    pub name: OsString,
    pub meta: Meta,
}
pub(crate) fn component(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes != b"."
        && bytes != b".."
        && !bytes.contains(&b'/')
        && !bytes.contains(&0)
}
pub(crate) fn meta(s: &libc::stat) -> Meta {
    let kind = match s.st_mode & libc::S_IFMT {
        libc::S_IFREG => Kind::File,
        libc::S_IFDIR => Kind::Directory,
        libc::S_IFLNK => Kind::Symlink,
        _ => Kind::Other,
    };
    Meta {
        kind,
        dev: s.st_dev as u64,
        ino: s.st_ino,
        links: s.st_nlink as u64,
        allocated: (s.st_blocks.max(0) as u64).saturating_mul(512),
        apparent: s.st_size.max(0) as u64,
    }
}
pub(crate) fn stat_fd(fd: RawFd) -> io::Result<Meta> {
    #[cfg(feature = "profile")]
    let _timer = crate::profile::Timer::new(crate::profile::Event::StatFd);
    let mut s = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat initializes stat on success; pointer is writable and fd is borrowed.
    if unsafe { libc::fstat(fd, s.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(meta(&unsafe { s.assume_init() }))
}
pub(crate) fn stat_at(fd: RawFd, name: &CStr) -> io::Result<Meta> {
    #[cfg(feature = "profile")]
    let _timer = crate::profile::Timer::new(crate::profile::Event::StatAt);
    let mut s = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: name is NUL terminated, buffer is writable; AT_SYMLINK_NOFOLLOW prevents target metadata reads.
    if unsafe { libc::fstatat(fd, name.as_ptr(), s.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(meta(&unsafe { s.assume_init() }))
}
pub(crate) fn open_at(fd: RawFd, name: &CStr) -> io::Result<OwnedFd> {
    #[cfg(feature = "profile")]
    let _timer = crate::profile::Timer::new(crate::profile::Event::Open);
    // SAFETY: name is NUL terminated. Only directories are opened, without following the final component.
    let out = unsafe {
        libc::openat(
            fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if out < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(out) })
    }
}

#[cfg(target_os = "macos")]
fn errno_ptr() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(target_os = "linux")]
fn errno_ptr() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}
pub(crate) fn portable(fd: RawFd, visit: &mut impl FnMut(Raw) -> bool) -> io::Result<u64> {
    // Open a fresh directory description: dup would share enumeration offsets with bulk reads.
    let fresh = open_at(fd, c".")?;
    use std::os::fd::IntoRawFd;
    let raw = fresh.into_raw_fd();
    // SAFETY: ownership of raw transfers to DIR on success; failure closes raw explicitly.
    let dir = unsafe { libc::fdopendir(raw) };
    if dir.is_null() {
        let e = io::Error::last_os_error();
        unsafe { libc::close(raw) };
        return Err(e);
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let _guard = Stream(dir);
    let mut errors = 0;
    loop {
        // SAFETY: errno is this thread's local errno; DIR is uniquely owned. d_name is NUL terminated by libc.
        unsafe { *errno_ptr() = 0 };
        let ent = {
            #[cfg(feature = "profile")]
            let _timer = crate::profile::Timer::new(crate::profile::Event::ReadDir);
            unsafe { libc::readdir(dir) }
        };
        if ent.is_null() {
            if unsafe { *errno_ptr() } != 0 {
                return Err(io::Error::last_os_error());
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        if !component(name.to_bytes()) {
            errors += 1;
            continue;
        }
        match stat_at(fd, name) {
            Ok(meta) => {
                if !visit(Raw {
                    name: OsString::from_vec(name.to_bytes().to_vec()),
                    meta,
                }) {
                    break;
                }
            }
            Err(_) => errors += 1,
        }
    }
    Ok(errors)
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    const NAME: u32 = 1;
    const DEVID: u32 = 2;
    const TYPE: u32 = 8;
    const ID: u32 = 0x02000000;
    const ERROR: u32 = 0x20000000;
    const RETURNED: u32 = 0x80000000;
    const LINKS: u32 = 1;
    const ALLOC: u32 = 4;
    const LENGTH: u32 = 0x200;
    const COMMON: u32 = NAME | DEVID | TYPE | ID | ERROR | RETURNED;
    const FILE: u32 = LINKS | ALLOC | LENGTH;
    fn bad() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "invalid bulk attribute record")
    }
    fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
        Ok(u32::from_ne_bytes(
            b.get(at..at.checked_add(4).ok_or_else(bad)?)
                .ok_or_else(bad)?
                .try_into()
                .map_err(|_| bad())?,
        ))
    }
    fn u64_at(b: &[u8], at: usize) -> io::Result<u64> {
        Ok(u64::from_ne_bytes(
            b.get(at..at.checked_add(8).ok_or_else(bad)?)
                .ok_or_else(bad)?
                .try_into()
                .map_err(|_| bad())?,
        ))
    }
    // FSOPT_PACK_INVAL_ATTRS fixes field offsets. Every validity bit is still checked before use.
    fn parse(b: &[u8]) -> io::Result<(OsString, Option<Meta>)> {
        if b.len() < 52 {
            return Err(bad());
        }
        let valid = u32_at(b, 4)?;
        let file = u32_at(b, 16)?;
        let offset = u32_at(b, 28)? as i32 as i64;
        let len = u32_at(b, 32)? as usize;
        let start = 28_i64.checked_add(offset).ok_or_else(bad)?;
        if valid & NAME == 0 || start < 52 || len < 2 {
            return Err(bad());
        }
        let start = usize::try_from(start).map_err(|_| bad())?;
        let end = start.checked_add(len).ok_or_else(bad)?;
        let name = b.get(start..end).ok_or_else(bad)?;
        if name.last() != Some(&0) || !component(&name[..name.len() - 1]) {
            return Err(bad());
        }
        let name = OsString::from_vec(name[..name.len() - 1].to_vec());
        if valid & ERROR != 0 && u32_at(b, 24)? != 0 {
            return Ok((name, None));
        }
        // Nonregular nodes and directories use stat: mount point bulk attributes describe the covered vnode.
        if valid & (DEVID | TYPE | ID) != (DEVID | TYPE | ID)
            || file & FILE != FILE
            || u32_at(b, 40)? != 1
        {
            return Ok((name, None));
        }
        if start < 72 {
            return Err(bad());
        }
        let allocated = u64_at(b, 56)?;
        let apparent = u64_at(b, 64)?;
        if allocated > i64::MAX as u64 || apparent > i64::MAX as u64 {
            return Err(bad());
        }
        // stat reports allocation in 512-byte blocks. Unusual filesystems may
        // vend an unrounded byte attribute; use authoritative stat in that case.
        if !allocated.is_multiple_of(512) {
            return Ok((name, None));
        }
        Ok((
            name,
            Some(Meta {
                kind: Kind::File,
                dev: u32_at(b, 36)? as libc::dev_t as u64,
                ino: u64_at(b, 44)?,
                links: u32_at(b, 52)? as u64,
                allocated,
                apparent,
            }),
        ))
    }
    pub(super) fn bulk(fd: RawFd, visit: &mut impl FnMut(Raw) -> bool) -> io::Result<u64> {
        let mut attr = libc::attrlist {
            bitmapcount: 5,
            reserved: 0,
            commonattr: COMMON,
            volattr: 0,
            dirattr: 0,
            fileattr: FILE,
            forkattr: 0,
        };
        // u64 storage ensures the native 8-byte buffer alignment requirement.
        let mut buffer = vec![0u64; 2048];
        let mut errors = 0;
        loop {
            // SAFETY: attr initialized, buffer aligned/writable for the advertised byte count; kernel never sees Rust references into it.
            let count = {
                #[cfg(feature = "profile")]
                let _timer = crate::profile::Timer::new(crate::profile::Event::Bulk);
                unsafe {
                    libc::getattrlistbulk(
                        fd,
                        (&mut attr as *mut libc::attrlist).cast(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len() * 8,
                        libc::FSOPT_PACK_INVAL_ATTRS as u64,
                    )
                }
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                return Ok(errors);
            }
            // SAFETY: initialized u64 buffer is readable as bytes; lifetime stays within this iteration.
            let bytes = unsafe {
                std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8)
            };
            let mut at = 0;
            for _ in 0..count {
                let len = u32_at(bytes, at)? as usize;
                if len < 52 || !len.is_multiple_of(8) {
                    return Err(bad());
                }
                let end = at.checked_add(len).ok_or_else(bad)?;
                let record = bytes.get(at..end).ok_or_else(bad)?;
                let (name, meta) = parse(record)?;
                at = end;
                let meta = match meta {
                    Some(m) => m,
                    None => match stat_at(
                        fd,
                        &CString::new(name.clone().into_vec()).map_err(|_| bad())?,
                    ) {
                        Ok(m) => m,
                        Err(_) => {
                            errors += 1;
                            continue;
                        }
                    },
                };
                if !visit(Raw { name, meta }) {
                    return Ok(errors);
                }
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn rejects_malformed_buffers() {
            for n in 0..128 {
                let b = vec![255; n];
                assert!(parse(&b).is_err());
            }
            let mut b = vec![0; 80];
            b[4..8].copy_from_slice(&NAME.to_ne_bytes());
            b[28..32].copy_from_slice(&i32::MIN.to_ne_bytes());
            b[32..36].copy_from_slice(&2u32.to_ne_bytes());
            assert!(parse(&b).is_err());
        }
        #[test]
        fn directory_record_omits_file_attributes() {
            let mut b = vec![0; 56];
            b[4..8].copy_from_slice(&COMMON.to_ne_bytes());
            b[28..32].copy_from_slice(&24u32.to_ne_bytes());
            b[32..36].copy_from_slice(&2u32.to_ne_bytes());
            b[40..44].copy_from_slice(&2u32.to_ne_bytes());
            b[52] = b'd';
            let (name, m) = parse(&b).unwrap();
            assert_eq!(name, "d");
            assert!(m.is_none());
        }
        #[test]
        fn file_device_sign_and_unaligned_allocation_fallback() {
            let mut b = vec![0u8; 80];
            b[4..8].copy_from_slice(&COMMON.to_ne_bytes());
            b[16..20].copy_from_slice(&FILE.to_ne_bytes());
            b[28..32].copy_from_slice(&44u32.to_ne_bytes());
            b[32..36].copy_from_slice(&2u32.to_ne_bytes());
            b[36..40].copy_from_slice(&(-7i32).to_ne_bytes());
            b[40..44].copy_from_slice(&1u32.to_ne_bytes());
            b[44..52].copy_from_slice(&1234u64.to_ne_bytes());
            b[52..56].copy_from_slice(&1u32.to_ne_bytes());
            b[56..64].copy_from_slice(&512u64.to_ne_bytes());
            b[64..72].copy_from_slice(&17u64.to_ne_bytes());
            b[72] = b'x';
            assert_eq!(parse(&b).unwrap().1.unwrap().dev, (-7i32) as u64);
            for allocation in [1u64, 511, 513, 4097] {
                b[56..64].copy_from_slice(&allocation.to_ne_bytes());
                assert!(
                    parse(&b).unwrap().1.is_none(),
                    "allocation {allocation} needs authoritative stat rounding"
                );
            }
        }
        #[test]
        fn bulk_parser_arbitrary_records_do_not_panic() {
            let mut state = 0x8ef92d6a101u64;
            for i in 0..50000 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let mut bytes = vec![0u8; (state as usize) % 256];
                for b in &mut bytes {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    *b = (state >> 32) as u8;
                }
                if i % 2 == 0 && bytes.len() >= 36 {
                    bytes[4..8].copy_from_slice(&COMMON.to_ne_bytes());
                }
                let _ = parse(&bytes);
            }
        }
    }
}

pub(crate) fn enumerate(
    fd: RawFd,
    bulk: bool,
    visit: &mut impl FnMut(Raw) -> bool,
) -> io::Result<u64> {
    #[cfg(target_os = "macos")]
    if bulk {
        return mac::bulk(fd, visit);
    }
    let _ = bulk;
    portable(fd, visit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn component_validation() {
        for s in [b"".as_slice(), b".", b"..", b"a/b", b"a\0"] {
            assert!(!component(s));
        }
        assert!(component(b"bad\xff\n\x1b"));
    }
}
