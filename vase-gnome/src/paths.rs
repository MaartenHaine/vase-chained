//! Where vase-gnome keeps its files, by the XDG base directories: the shared vase config, its state,
//! and its socket and lock under the runtime dir (protocol §1). M5 settles the config path with the
//! other platforms; this is the minimum M1a needs.

use std::path::{Path, PathBuf};

use vase_core::daemon::Paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// `$XDG_CONFIG_HOME/vase`: read only.
    pub config: PathBuf,
    /// `$XDG_STATE_HOME/vase`: `state.json`.
    pub state: PathBuf,
    /// `$XDG_RUNTIME_DIR/vase`, 0700: the socket and the lock.
    pub runtime: PathBuf,
}

impl Dirs {
    /// From the environment. `XDG_RUNTIME_DIR` has no fallback: without it there is no session to
    /// serve, and a socket anywhere else would be outside the user's private runtime dir.
    pub fn from_env() -> Result<Dirs, String> {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
        let home = var("HOME");
        let base = |xdg: &str, fallback: &str| var(xdg).or_else(|| home.as_ref().map(|h| h.join(fallback)));
        let config = base("XDG_CONFIG_HOME", ".config").ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
        let state = base("XDG_STATE_HOME", ".local/state").ok_or("neither XDG_STATE_HOME nor HOME is set")?;
        let runtime = var("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
        Ok(Dirs { config: config.join("vase"), state: state.join("vase"), runtime: runtime.join("vase") })
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime.join("gnome.sock")
    }

    pub fn lock(&self) -> PathBuf {
        self.runtime.join("gnome.lock")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.state.join("state.json")
    }

    pub fn paths(&self) -> Paths {
        Paths { config: Some(self.config_file()), state: Some(self.state_file()) }
    }
}

/// The runtime dir for the socket: created 0700 if missing, and made 0700 if it is ours.
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != crate::peer::own_uid() {
        return Err(std::io::Error::other(format!("{} is not a directory this user owns", dir.display())));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}
