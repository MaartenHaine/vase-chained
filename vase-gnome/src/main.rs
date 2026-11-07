//! `vase-gnome [--shell-exe PATH]`: the Linux Tiler's daemon, started by its `systemd --user` unit.
//! `--shell-exe` names the binary the connecting compositor must run from (default
//! `/usr/bin/gnome-shell`); the L0 tests point it at their own fake extension.

#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(run());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("vase-gnome runs on Linux only");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn run() -> i32 {
    use std::path::PathBuf;

    use vase_gnome::host::{Host, Settings};
    use vase_gnome::paths::Dirs;
    use vase_gnome::server::{self, Bind};

    let mut shell_exe = PathBuf::from(vase_gnome::SHELL_EXE);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--shell-exe" => match args.next() {
                Some(p) => shell_exe = PathBuf::from(p),
                None => return usage(),
            },
            "--version" => {
                println!("vase-gnome {} (build {})", env!("CARGO_PKG_VERSION"), vase_gnome::BUILD);
                return 0;
            }
            _ => return usage(),
        }
    }

    let dirs = match Dirs::from_env() {
        Ok(dirs) => dirs,
        Err(e) => {
            eprintln!("vase-gnome: {e}");
            return 1;
        }
    };
    let bound = match server::bind(&dirs) {
        Ok(Bind::Bound(bound)) => bound,
        Ok(Bind::Held) => {
            eprintln!("vase-gnome: another vase-gnome holds {}; leaving its socket alone", dirs.lock().display());
            return 0;
        }
        Err(e) => {
            eprintln!("vase-gnome: could not bind {}: {e}", dirs.socket().display());
            return 1;
        }
    };
    if let Err(e) = server::install_signals() {
        eprintln!("vase-gnome: could not install the SIGTERM handler: {e}");
        return 1;
    }
    let settings = Settings { paths: dirs.paths(), build: vase_gnome::BUILD.to_string(), mdwe: vase_gnome::mdwe(), landlock: None };
    server::serve(bound, Host::new(settings), &shell_exe)
}

#[cfg(target_os = "linux")]
fn usage() -> i32 {
    eprintln!("usage: vase-gnome [--shell-exe PATH] [--version]");
    2
}
