//! `vase-gnome`: the Linux Tiler's daemon, started by its `systemd --user` unit.

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
    use vase_gnome::host::{Host, Settings};
    use vase_gnome::paths::Dirs;
    use vase_gnome::server::{self, Bind};

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [v] if v == "--version" => {
            println!("vase-gnome {} (build {})", env!("CARGO_PKG_VERSION"), vase_gnome::BUILD);
            return 0;
        }
        _ => return usage(),
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
    // Confined once the socket is bound and before anything is read from it.
    let landlock = match vase_gnome::confine::apply(&dirs) {
        Ok(status) => status,
        Err(e) => {
            eprintln!("vase-gnome: Landlock not applied: {e}");
            "off"
        }
    };
    eprintln!("vase-gnome: Landlock: {landlock}");
    let settings = Settings { paths: dirs.paths(), build: vase_gnome::BUILD.to_string(), mdwe: vase_gnome::mdwe(), landlock: Some(landlock.to_string()) };
    server::serve(bound, Host::new(settings))
}

#[cfg(target_os = "linux")]
fn usage() -> i32 {
    eprintln!("usage: vase-gnome [--version]");
    2
}
