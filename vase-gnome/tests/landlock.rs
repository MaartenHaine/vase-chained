//! Landlock (wayfinder §3.6, Q2.3): once confined, the daemon reads its config, reads and writes its
//! state and runtime dirs, and can do nothing else with files: opening a file elsewhere and an
//! `execve` both fail, and so does a TCP socket. The confinement is irreversible, so the checks run
//! in a child: this test binary again, with `VG_LANDLOCK_CHILD` naming the dirs.
#![cfg(target_os = "linux")]

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use vase_gnome::paths::Dirs;

const CHILD: &str = "VG_LANDLOCK_CHILD";

/// The kernel's Landlock ABI, or 0 if it has none (or a seccomp filter hides it).
fn kernel_abi() -> i64 {
    const LANDLOCK_CREATE_RULESET_VERSION: libc::c_ulong = 1;
    // SAFETY: the version query takes no ruleset and writes nothing.
    let v = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<libc::c_void>(), 0usize, LANDLOCK_CREATE_RULESET_VERSION) };
    v.max(0)
}

fn dirs(root: &Path) -> Dirs {
    Dirs { config: root.join("config/vase"), state: root.join("state/vase"), runtime: root.join("run/vase") }
}

#[test]
fn confined_the_daemon_touches_only_its_own_dirs_and_execs_nothing() {
    if let Some(root) = std::env::var_os(CHILD) {
        child(Path::new(&root));
        return;
    }
    let root = std::env::temp_dir().join(format!("vg-landlock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let d = dirs(&root);
    std::fs::create_dir_all(&d.runtime).unwrap();
    std::fs::write(root.join("outside.txt"), "secret").unwrap();
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "confined_the_daemon_touches_only_its_own_dirs_and_execs_nothing", "--nocapture", "--test-threads=1"])
        .env(CHILD, &root)
        .output()
        .unwrap();
    let report = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{report}");
    // Full from the ABI the confinement targets (`confine::ABI`, 8) up; below it, what the kernel knows.
    let abi = kernel_abi();
    if abi >= 8 {
        assert!(report.contains("landlock: full"), "the kernel has Landlock ABI {abi}, so it must hold fully:\n{report}");
    } else if abi > 0 {
        assert!(report.contains("landlock: partial"), "the kernel has Landlock ABI {abi} (< 8), so it must hold partially:\n{report}");
    } else {
        eprintln!("no Landlock in this kernel or container: only the best-effort path was checked\n{report}");
    }
    assert!(abi == 0 || report.contains("every denial held"), "{report}");
}

fn denied(r: std::io::Result<impl Sized>, what: &str) {
    match r {
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {}
        Err(e) => panic!("{what}: denied with the wrong error: {e}"),
        Ok(_) => panic!("{what}: allowed"),
    }
}

fn child(root: &Path) {
    let d = dirs(root);
    let status = vase_gnome::confine::apply(&d).expect("the ruleset builds");
    println!("landlock: {status}");
    // Its own dirs work as before.
    assert!(std::fs::read_to_string(d.config_file()).unwrap().contains("[tabbar]") || d.config_file().exists());
    std::fs::write(d.state_file(), "{}").unwrap();
    std::fs::write(d.runtime.join("probe"), "x").unwrap();
    std::fs::remove_file(d.runtime.join("probe")).unwrap();
    if status == "off" {
        return;
    }
    let outside: PathBuf = root.join("outside.txt");
    denied(std::fs::read_to_string(&outside), "reading a file outside its dirs");
    denied(std::fs::read_to_string("/etc/passwd"), "reading /etc/passwd");
    denied(std::fs::write(root.join("new.txt"), "x"), "writing outside its dirs");
    denied(std::fs::write(d.config_file(), "x"), "writing its config");
    denied(Command::new("/bin/true").status(), "an execve");
    denied(Command::new(std::env::current_exe().unwrap()).arg("--list").status(), "an execve of its own binary");
    // Landlock handles TCP from ABI 4.
    if kernel_abi() >= 4 {
        denied(std::net::TcpListener::bind("127.0.0.1:0"), "a TCP bind");
    }
    println!("landlock child: every denial held");
}
