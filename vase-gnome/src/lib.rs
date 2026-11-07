//! vase-gnome: the Linux Tiler's daemon. It runs vase-core's `Daemon` for the GNOME Shell extension,
//! over protocol v1 on a unix socket it binds itself (`docs/linux-tiler/protocol-v1.md`). Linux only:
//! on other platforms the crate is empty.
#![cfg(target_os = "linux")]

pub mod backend;
pub mod confine;
pub mod host;
pub mod keys;
pub mod mirror;
pub mod painter;
pub mod paths;
pub mod peer;
pub mod protocol;
pub mod server;

/// The build hash of the reviewed SHA, set by the build (Chained's sudo step compares it, §4).
pub const BUILD: &str = match option_env!("VASE_GNOME_BUILD") {
    Some(b) => b,
    None => "dev",
};

/// Whether the kernel refuses this process memory that is writable and executable
/// (`MemoryDenyWriteExecute=`, which systemd sets with `PR_SET_MDWE`; `/proc` doesn't show it, S10).
pub fn mdwe() -> bool {
    const PR_GET_MDWE: libc::c_int = 66;
    const PR_MDWE_REFUSE_EXEC_GAIN: libc::c_int = 1;
    // SAFETY: PR_GET_MDWE takes no pointers; an older kernel answers EINVAL.
    let flags = unsafe { libc::prctl(PR_GET_MDWE, 0, 0, 0, 0) };
    flags > 0 && flags & PR_MDWE_REFUSE_EXEC_GAIN != 0
}
