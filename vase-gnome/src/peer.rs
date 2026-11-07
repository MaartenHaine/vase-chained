//! The peer check on the socket (protocol §1, S9): `SO_PEERCRED` names the process at the other end,
//! its uid must be ours, and `/proc/<pid>/exe` must be the expected binary. The extension runs the
//! whole check on the daemon (the reference for M1b is `judge` with `replaced_ok: false`). The
//! daemon checks only the uid (`check_uid`): confined by Landlock it may not read the exe of a
//! process outside its domain, which the unconfined shell is (Landlock scopes ptrace access, and
//! `/proc/<pid>/exe` needs it). It is defence in depth either way: any same-uid process can reach
//! the compositor in other ways (`devtest-security.md` §5.5).

use std::fmt;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// The suffix the kernel puts on `/proc/<pid>/exe` once the file was replaced under the process.
const DELETED: &str = " (deleted)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cred {
    pub pid: i32,
    pub uid: u32,
}

/// The credentials of the process at the other end of `stream`, as the kernel recorded them at
/// `connect` (or, for the client, at `listen`).
pub fn cred(stream: &UnixStream) -> io::Result<Cred> {
    let mut ucred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `ucred` is a valid, writable `struct ucred` and `len` holds its size, as SO_PEERCRED wants.
    let rc = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut ucred as *mut libc::ucred).cast(), &mut len) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Cred { pid: ucred.pid, uid: ucred.uid })
}

pub fn own_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Why a peer was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Uid {
        got: u32,
        want: u32,
    },
    /// `/proc/<pid>/exe` could not be read: an execute-only binary, or a non-dumpable process (S9).
    Unreadable(String),
    Exe {
        got: PathBuf,
        want: PathBuf,
    },
    /// The binary was replaced under the running process (an upgrade without a restart).
    Replaced(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Refusal::Uid { got, want } => write!(f, "peer uid {got}, expected {want}"),
            Refusal::Unreadable(e) => write!(f, "peer exe unreadable: {e}"),
            Refusal::Exe { got, want } => write!(f, "peer exe {}, expected {}", got.display(), want.display()),
            Refusal::Replaced(p) => write!(f, "{} was replaced under the running process; restart it", p.display()),
        }
    }
}

/// The decision, apart from the system calls so it can be tested for any uid. `exe` is what
/// `readlink /proc/<pid>/exe` gave. With `replaced_ok`, a binary replaced under a running process
/// still passes (the daemon allows that for the shell, which an apt upgrade replaces while the
/// session runs); the extension refuses it for the daemon (S9 item 6).
pub fn judge(cred: Cred, own_uid: u32, exe: io::Result<PathBuf>, want: &Path, replaced_ok: bool) -> Result<(), Refusal> {
    if cred.uid != own_uid {
        return Err(Refusal::Uid { got: cred.uid, want: own_uid });
    }
    let exe = exe.map_err(|e| Refusal::Unreadable(e.to_string()))?;
    let text = exe.to_string_lossy();
    if let Some(original) = text.strip_suffix(DELETED) {
        let original = PathBuf::from(original);
        if original == want && replaced_ok {
            return Ok(());
        }
        if original == want {
            return Err(Refusal::Replaced(original));
        }
        return Err(Refusal::Exe { got: exe, want: want.to_path_buf() });
    }
    if exe != want {
        return Err(Refusal::Exe { got: exe, want: want.to_path_buf() });
    }
    Ok(())
}

/// The daemon's check on the extension: the uid only (see the module doc).
pub fn check_uid(stream: &UnixStream) -> Result<Cred, Refusal> {
    let cred = cred(stream).map_err(|e| Refusal::Unreadable(format!("SO_PEERCRED: {e}")))?;
    let want = own_uid();
    if cred.uid != want {
        return Err(Refusal::Uid { got: cred.uid, want });
    }
    Ok(cred)
}

/// Check the process at the other end of `stream`.
pub fn check(stream: &UnixStream, want: &Path, replaced_ok: bool) -> Result<Cred, Refusal> {
    let cred = cred(stream).map_err(|e| Refusal::Unreadable(format!("SO_PEERCRED: {e}")))?;
    let exe = std::fs::read_link(format!("/proc/{}/exe", cred.pid));
    judge(cred, own_uid(), exe, want, replaced_ok).map(|()| cred)
}

#[cfg(test)]
#[path = "../tests/unit/peer.rs"]
mod tests;
