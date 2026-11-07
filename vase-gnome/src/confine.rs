//! Landlock self-confinement (wayfinder §3.6, Q2.3): once the socket is bound, the daemon restricts
//! itself to reading its config, reading and writing its state and runtime dirs, and nothing else:
//! no other file, no exec anywhere, no TCP, no abstract unix socket outside its own domain, no
//! signal to another process. It needs no root, so it holds on every Linux device the owner uses and
//! when the AppArmor profile is missing or doesn't attach (F35). Best effort: an older kernel
//! enforces what it knows, and the hello reports how much held (`"full"`, `"partial"`, `"off"`).
//!
//! The config dir is read-only on purpose: vase writes `config.toml` only to create the default
//! (done before confining) and to save picker favorites, which fail quietly here (M5 decides).

use landlock::{path_beneath_rules, Access, AccessFs, AccessNet, BitFlags, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetError, RulesetStatus, Scope, ABI};

use crate::paths::Dirs;

/// The ABI the confinement is designed and tested for: the owner's kernel (7.0, ABI 8, S10). A newer
/// kernel enforces all of it ("full"); an older one what it knows ("partial"). ABI 9 adds a rule for
/// connecting to pathname unix sockets, which the daemon never does; moving up is a reviewed step.
const ABI: ABI = ABI::V8;

/// What reading means here: files and directory listings, never exec.
fn read() -> BitFlags<AccessFs> {
    AccessFs::ReadFile | AccessFs::ReadDir
}

/// Everything but exec.
fn read_write() -> BitFlags<AccessFs> {
    AccessFs::from_all(ABI) & !BitFlags::from(AccessFs::Execute)
}

/// Make the dirs the rules name (a rule needs the path to exist), then confine the calling thread
/// and every thread and process it starts afterwards. Call it before any other thread exists.
pub fn apply(dirs: &Dirs) -> Result<&'static str, RulesetError> {
    // Created while that is still allowed: the default config.toml, and the state dir.
    vase_core::config::Config::ensure(&dirs.config_file());
    let _ = std::fs::create_dir_all(&dirs.state);
    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(ABI))?
        .handle_access(AccessNet::from_all(ABI))?
        .scope(Scope::from_all(ABI))?
        .create()?
        .add_rules(path_beneath_rules([&dirs.config], read()))?
        .add_rules(path_beneath_rules([&dirs.state, &dirs.runtime], read_write()))?
        .restrict_self()?;
    Ok(match status.ruleset {
        RulesetStatus::FullyEnforced => "full",
        RulesetStatus::PartiallyEnforced => "partial",
        RulesetStatus::NotEnforced => "off",
    })
}
