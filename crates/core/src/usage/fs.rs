//! Descriptor-relative, read-only access. No symlinks, hardlinked files, special
//! files, recursive walks, permission changes, or caller-supplied IPC paths.
use std::ffi::{CStr, CString, OsStr};
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

pub(super) fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Unsupported local usage data")
}

pub(super) struct Dir(File);
impl Dir {
    pub fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(invalid());
        }
        let mut dir = Self(open_at(libc::AT_FDCWD, OsStr::new("/"), true)?);
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => {
                    dir = dir.dir(name)?;
                }
                _ => return Err(invalid()),
            }
        }
        Ok(dir)
    }
    pub fn dir(&self, name: &OsStr) -> io::Result<Self> {
        Ok(Self(open_at(self.0.as_raw_fd(), name, true)?))
    }
    pub fn file(&self, name: &OsStr) -> io::Result<File> {
        let file = open_at(self.0.as_raw_fd(), name, false)?;
        let meta = file.metadata()?;
        // SAFETY: geteuid has no arguments or memory accesses.
        if !meta.is_file() || meta.nlink() != 1 || meta.uid() != unsafe { libc::geteuid() } {
            return Err(invalid());
        }
        Ok(file)
    }
    pub fn read(&self, name: &str, limit: usize) -> io::Result<Vec<u8>> {
        let mut file = self.file(OsStr::new(name))?;
        let before = file.metadata()?;
        if before.len() > limit as u64 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit || !stable(&before, &file.metadata()?) {
            return Err(invalid());
        }
        Ok(bytes)
    }
    pub fn names(&self, limit: usize) -> io::Result<Vec<String>> {
        // Open a new directory description so readdir's offset is independent.
        let stream_file = open_at(self.0.as_raw_fd(), OsStr::new("."), true)?;
        // SAFETY: dup yields an independently owned valid fd for fdopendir.
        let fd = unsafe { libc::dup(stream_file.as_raw_fd()) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is owned and refers to a directory. fdopendir takes ownership on success.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            // SAFETY: fdopendir failed, leaving this fd owned here.
            unsafe {
                libc::close(fd);
            }
            return Err(io::Error::last_os_error());
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                // SAFETY: this wrapper uniquely owns the open DIR.
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let stream = Stream(stream);
        let mut names = Vec::new();
        let mut seen = 0;
        loop {
            // SAFETY: the errno pointer belongs to the current thread.
            unsafe {
                *errno_ptr() = 0;
            }
            // SAFETY: stream is valid and is accessed only on this thread.
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                // SAFETY: errno is thread-local, checked immediately after readdir.
                if unsafe { *errno_ptr() } != 0 {
                    return Err(io::Error::last_os_error());
                }
                break;
            }
            // SAFETY: d_name is a NUL-terminated name, valid until the next readdir.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            seen += 1;
            if seen > limit {
                return Err(invalid());
            }
            // Skip undecodable filesystem names; never turn them into lossy paths.
            if let Ok(name) = name.to_str() {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }
}

fn open_at(fd: libc::c_int, name: &OsStr, directory: bool) -> io::Result<File> {
    let name = CString::new(name.as_bytes()).map_err(|_| invalid())?;
    if name.as_bytes().contains(&b'/') && name.as_bytes() != b"/" {
        return Err(invalid());
    }
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: name is NUL-terminated; no creation flags, vararg mode is not required.
    let opened = unsafe { libc::openat(fd, name.as_ptr(), flags) };
    if opened < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new fd, now uniquely owned by File.
    Ok(unsafe { File::from_raw_fd(opened) })
}

pub(super) fn stable(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

#[cfg(target_os = "macos")]
fn errno_ptr() -> *mut libc::c_int {
    // SAFETY: __error returns this thread's errno location.
    unsafe { libc::__error() }
}
#[cfg(not(target_os = "macos"))]
fn errno_ptr() -> *mut libc::c_int {
    // SAFETY: __errno_location returns this thread's errno location on Linux.
    unsafe { libc::__errno_location() }
}
