//! Filesystem abstraction for file-test primaries.
//!
//! The trait deliberately mirrors what `[[`'s primaries need: stat (with and
//! without symlink follow), an access check for `-r`/`-w`/`-x`, and a tty
//! check for `-t`. The [`StdFs`] impl uses `std::fs` and `libc` on unix.

use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Regular,
    Directory,
    Symlink,
    BlockDevice,
    CharDevice,
    NamedPipe,
    Socket,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub struct FileStat {
    pub kind: FileKind,
    pub size: u64,
    /// Lower 12 bits of mode (permissions + setuid/setgid/sticky).
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub dev: u64,
    pub ino: u64,
    /// Modification time as (sec, nsec).
    pub mtime: (i64, i64),
    /// Access time as (sec, nsec).
    pub atime: (i64, i64),
}

#[derive(Debug, Clone, Copy)]
pub enum AccessMode {
    Read,
    Write,
    Execute,
}

pub trait FileSystem {
    /// Stat following symlinks.
    fn stat(&self, path: &Path) -> io::Result<FileStat>;

    /// Stat without following symlinks (used for `-h` / `-L`).
    fn lstat(&self, path: &Path) -> io::Result<FileStat>;

    /// Effective-uid access check.
    fn access(&self, path: &Path, mode: AccessMode) -> bool;

    /// Whether `fd` is a TTY. Used by `-t`.
    fn is_tty(&self, fd: i32) -> bool;

    /// Effective UID of the running process (for `-O`).
    fn effective_uid(&self) -> u32;

    /// Effective GID of the running process (for `-G`).
    fn effective_gid(&self) -> u32;
}

/// Default filesystem impl using `std::fs` and (on unix) `libc`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdFs;

#[cfg(unix)]
mod unix_impl {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn kind_from_mode(mode: u32) -> FileKind {
        const S_IFMT: u32 = 0o170000;
        const S_IFREG: u32 = 0o100000;
        const S_IFDIR: u32 = 0o040000;
        const S_IFLNK: u32 = 0o120000;
        const S_IFBLK: u32 = 0o060000;
        const S_IFCHR: u32 = 0o020000;
        const S_IFIFO: u32 = 0o010000;
        const S_IFSOCK: u32 = 0o140000;
        match mode & S_IFMT {
            S_IFREG => FileKind::Regular,
            S_IFDIR => FileKind::Directory,
            S_IFLNK => FileKind::Symlink,
            S_IFBLK => FileKind::BlockDevice,
            S_IFCHR => FileKind::CharDevice,
            S_IFIFO => FileKind::NamedPipe,
            S_IFSOCK => FileKind::Socket,
            _ => FileKind::Other,
        }
    }

    fn to_stat(md: std::fs::Metadata) -> FileStat {
        let mode = md.mode();
        FileStat {
            kind: kind_from_mode(mode),
            size: md.size(),
            mode: mode & 0o7777,
            uid: md.uid(),
            gid: md.gid(),
            dev: md.dev(),
            ino: md.ino(),
            mtime: (md.mtime(), md.mtime_nsec()),
            atime: (md.atime(), md.atime_nsec()),
        }
    }

    impl FileSystem for StdFs {
        fn stat(&self, path: &Path) -> io::Result<FileStat> {
            std::fs::metadata(path).map(to_stat)
        }
        fn lstat(&self, path: &Path) -> io::Result<FileStat> {
            std::fs::symlink_metadata(path).map(to_stat)
        }
        fn access(&self, path: &Path, mode: AccessMode) -> bool {
            use std::ffi::CString;
            let Ok(c) = CString::new(path.as_os_str().as_encoded_bytes()) else {
                return false;
            };
            let m = match mode {
                AccessMode::Read => libc::R_OK,
                AccessMode::Write => libc::W_OK,
                AccessMode::Execute => libc::X_OK,
            };
            // Safety: c is a valid C string; libc::access is safe to call.
            unsafe { libc::access(c.as_ptr(), m) == 0 }
        }
        fn is_tty(&self, fd: i32) -> bool {
            // Safety: just calls libc::isatty(int).
            unsafe { libc::isatty(fd) != 0 }
        }
        fn effective_uid(&self) -> u32 {
            // Safety: parameterless syscall.
            unsafe { libc::geteuid() }
        }
        fn effective_gid(&self) -> u32 {
            unsafe { libc::getegid() }
        }
    }
}

#[cfg(not(unix))]
mod fallback_impl {
    use super::*;

    impl FileSystem for StdFs {
        fn stat(&self, path: &Path) -> io::Result<FileStat> {
            let md = std::fs::metadata(path)?;
            Ok(FileStat {
                kind: if md.is_dir() {
                    FileKind::Directory
                } else if md.is_file() {
                    FileKind::Regular
                } else {
                    FileKind::Other
                },
                size: md.len(),
                mode: 0,
                uid: 0,
                gid: 0,
                dev: 0,
                ino: 0,
                mtime: (0, 0),
                atime: (0, 0),
            })
        }
        fn lstat(&self, path: &Path) -> io::Result<FileStat> {
            self.stat(path)
        }
        fn access(&self, path: &Path, mode: AccessMode) -> bool {
            let Ok(md) = std::fs::metadata(path) else {
                return false;
            };
            match mode {
                AccessMode::Read => true,
                AccessMode::Write => !md.permissions().readonly(),
                AccessMode::Execute => false,
            }
        }
        fn is_tty(&self, _fd: i32) -> bool {
            false
        }
        fn effective_uid(&self) -> u32 {
            0
        }
        fn effective_gid(&self) -> u32 {
            0
        }
    }
}
