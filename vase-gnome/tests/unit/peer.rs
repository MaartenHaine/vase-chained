//! The peer check's decision, for uids and binaries a test process can't be (protocol §1, S9).

use super::*;

const ME: u32 = 1000;

fn cred(uid: u32) -> Cred {
    Cred { pid: 42, uid }
}

fn exe(p: &str) -> io::Result<PathBuf> {
    Ok(PathBuf::from(p))
}

#[test]
fn the_expected_binary_under_our_uid_passes() {
    assert_eq!(judge(cred(ME), ME, exe("/usr/local/libexec/vase/vase-gnome"), Path::new("/usr/local/libexec/vase/vase-gnome"), false), Ok(()));
}

#[test]
fn another_uid_is_refused_before_anything_else() {
    assert_eq!(judge(cred(0), ME, exe("/usr/bin/gnome-shell"), Path::new("/usr/bin/gnome-shell"), true), Err(Refusal::Uid { got: 0, want: ME }));
}

#[test]
fn another_binary_is_refused() {
    let r = judge(cred(ME), ME, exe("/usr/bin/python3.14"), Path::new("/usr/local/libexec/vase/vase-gnome"), false);
    assert!(matches!(r, Err(Refusal::Exe { .. })), "{r:?}");
}

#[test]
fn a_replaced_daemon_is_refused_with_a_restart_hint_but_a_replaced_shell_passes() {
    let daemon = Path::new("/usr/local/libexec/vase/vase-gnome");
    let r = judge(cred(ME), ME, exe("/usr/local/libexec/vase/vase-gnome (deleted)"), daemon, false);
    assert_eq!(r, Err(Refusal::Replaced(daemon.to_path_buf())));
    assert!(r.unwrap_err().to_string().contains("restart"));
    let shell = Path::new("/usr/bin/gnome-shell");
    assert_eq!(judge(cred(ME), ME, exe("/usr/bin/gnome-shell (deleted)"), shell, true), Ok(()));
    // Another binary that happens to be deleted is still another binary.
    assert!(matches!(judge(cred(ME), ME, exe("/tmp/x (deleted)"), shell, true), Err(Refusal::Exe { .. })));
}

#[test]
fn an_unreadable_exe_is_refused() {
    let r = judge(cred(ME), ME, Err(io::Error::from(io::ErrorKind::PermissionDenied)), Path::new("/usr/bin/gnome-shell"), true);
    assert!(matches!(r, Err(Refusal::Unreadable(_))));
}
