//! The socket (protocol §1, §2): the lock before the bind, one connection at a time, the peer check
//! on accept, reads in chunks under the line cap, a capped write queue, SIGTERM, and the 25 ms tick.
//! One thread and `poll(2)`; every decision is `Host`'s.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::host::{Exit, Host, Output, TICK};
use crate::paths::{private_dir, Dirs};
use crate::peer;
use crate::protocol::{LineReader, TooLong, CHUNK, MAX_LINE};

/// The listening socket, with the lock that makes it this daemon's.
pub struct Bound {
    /// Held open for the process's life: the `flock` goes with it.
    _lock: File,
    pub listener: UnixListener,
    pub socket: PathBuf,
}

pub enum Bind {
    Bound(Bound),
    /// Another daemon holds the lock: this one must leave the socket alone and exit.
    Held,
}

fn flock_exclusive(file: &File) -> io::Result<bool> {
    // SAFETY: a plain flock on an fd this process owns.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(e)
    }
}

/// Take the lock, then (holding it) remove a stale socket a killed daemon left, and bind (S9 item 5).
pub fn bind(dirs: &Dirs) -> io::Result<Bind> {
    private_dir(&dirs.runtime)?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(dirs.lock())?;
    if !flock_exclusive(&lock)? {
        return Ok(Bind::Held);
    }
    let socket = dirs.socket();
    match std::fs::symlink_metadata(&socket) {
        Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(&socket)?,
        Ok(_) => return Err(io::Error::other(format!("{} exists and is not a socket", socket.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(Bind::Bound(Bound { _lock: lock, listener, socket }))
}

static TERM: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_signal: libc::c_int) {
    TERM.store(true, Ordering::SeqCst);
}

/// SIGTERM (and SIGINT, for a run from a terminal) set a flag the loop reads. No `SA_RESTART`, so the
/// signal also cuts the `poll` short.
pub fn install_signals() -> io::Result<()> {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        // SAFETY: a zeroed sigaction with a handler that only stores to an atomic, which is
        // async-signal-safe.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_term as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

/// The live connection's bytes.
struct Conn {
    stream: UnixStream,
    reader: LineReader,
    out: VecDeque<u8>,
    /// Close once `out` is written.
    closing: bool,
}

impl Conn {
    fn queue(&mut self, output: Output) {
        for line in output.lines {
            self.out.extend(line.as_bytes());
        }
        self.closing |= output.close;
    }

    /// Write what the socket takes now. Err means the extension is gone.
    fn write_some(&mut self) -> io::Result<()> {
        while !self.out.is_empty() {
            let (front, _) = self.out.as_slices();
            match self.stream.write(front) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.out.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// The last lines before a close: a short blocking write, then the socket is shut.
    fn finish(mut self) {
        let _ = self.stream.set_nonblocking(false);
        let _ = self.stream.set_write_timeout(Some(Duration::from_millis(200)));
        let (a, b) = self.out.as_slices();
        let _ = self.stream.write_all(a).and_then(|()| self.stream.write_all(b));
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// Turn away a second extension while one is live (§1), without waiting on it.
fn refuse_busy(stream: UnixStream) {
    let mut stream = stream;
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let _ = stream.write_all(Host::busy_line().as_bytes());
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn poll(fds: &mut [libc::pollfd], timeout: Duration) -> io::Result<()> {
    let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
    // SAFETY: `fds` is a valid, writable slice of pollfd for its whole length.
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    Ok(())
}

/// Serve until SIGTERM or a quit. Returns the exit code.
pub fn serve(bound: Bound, mut host: Host) -> i32 {
    let Bound { listener, socket, .. } = &bound;
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("vase-gnome: {e}");
        return 1;
    }
    eprintln!("vase-gnome: listening on {}", socket.display());
    let mut conn: Option<Conn> = None;
    let mut next_tick = Instant::now() + TICK;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if TERM.load(Ordering::SeqCst) {
            eprintln!("vase-gnome: SIGTERM; saving the layout and letting go, nothing moves");
            let output = host.terminate();
            if let Some(mut c) = conn.take() {
                c.queue(output);
                c.finish();
            }
            let _ = std::fs::remove_file(socket);
            return 0;
        }

        let mut fds = vec![libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        if let Some(c) = &conn {
            let events = libc::POLLIN | if c.out.is_empty() { 0 } else { libc::POLLOUT };
            fds.push(libc::pollfd { fd: c.stream.as_raw_fd(), events, revents: 0 });
        }
        if let Err(e) = poll(&mut fds, next_tick.saturating_duration_since(Instant::now())) {
            eprintln!("vase-gnome: poll: {e}");
            return 1;
        }

        // A live extension that has hung up (a rebase, a disable then enable) is read to its EOF first,
        // and its successor waits in the backlog meanwhile instead of being refused as busy.
        let hung_up = fds.get(1).is_some_and(|p| p.revents & libc::POLLHUP != 0);
        if fds[0].revents & libc::POLLIN != 0 && !hung_up {
            loop {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        eprintln!("vase-gnome: accept: {e}");
                        break;
                    }
                };
                if let Err(refusal) = peer::check_uid(&stream) {
                    eprintln!("vase-gnome: refusing a connection: {refusal}");
                    continue;
                }
                if conn.is_some() {
                    eprintln!("vase-gnome: a second extension connected while one is live; refused (busy)");
                    refuse_busy(stream);
                    continue;
                }
                if stream.set_nonblocking(true).is_err() {
                    continue;
                }
                let mut c = Conn { stream, reader: LineReader::default(), out: VecDeque::new(), closing: false };
                c.queue(host.accept(Instant::now()));
                conn = Some(c);
            }
        }

        let mut gone = false;
        if let Some(c) = &mut conn {
            if !c.closing {
                match c.stream.read(&mut buf) {
                    Ok(0) => gone = true,
                    Ok(n) => match c.reader.push(&buf[..n]) {
                        Ok(lines) => {
                            for line in lines {
                                c.queue(host.receive(&line, Instant::now()));
                                if c.closing {
                                    break;
                                }
                            }
                        }
                        Err(TooLong) => {
                            eprintln!("vase-gnome: refusing the extension: a line over {MAX_LINE} bytes");
                            c.queue(host.too_long());
                        }
                    },
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {}
                    Err(_) => gone = true,
                }
            }
            if !gone && c.write_some().is_err() {
                gone = true;
            }
            // An extension that stops reading is as good as gone (the same cap it keeps, §2).
            if c.out.len() > MAX_LINE {
                eprintln!("vase-gnome: the extension stopped reading; dropping it");
                gone = true;
            }
        }

        let now = Instant::now();
        if now >= next_tick {
            next_tick = (next_tick + TICK).max(now);
            if let Some(c) = &mut conn {
                c.queue(host.tick(now));
            }
        }

        if gone {
            eprintln!("vase-gnome: the extension went away; frozen until it is back");
            conn = None;
            host.eof();
        } else if conn.as_ref().is_some_and(|c| c.closing) {
            conn.take().unwrap().finish();
            host.eof();
        }

        if host.exit() == Some(Exit::Quit) && conn.is_none() {
            eprintln!("vase-gnome: quit; every window is back where it was");
            let _ = std::fs::remove_file(socket);
            return 0;
        }
    }
}
