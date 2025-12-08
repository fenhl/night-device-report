#![allow(unused_crate_dependencies)] // combined lib/bin crate

use {
    std::{
        cmp::Ordering::*,
        io::prelude::*,
    },
    gix_hash::ObjectId,
    lazy_regex::regex_captures,
    std::collections::HashMap,
    semver::Version,
    serde::{
        Deserialize,
        Serialize,
    },
    tokio::{
        io,
        process::Command,
    },
    unicode_width::UnicodeWidthStr as _,
    wheel::traits::{
        AsyncCommandOutputExt as _,
        CommandExt as _,
        IoResultExt as _,
    },
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "async-proto", derive(async_proto::Protocol))]
#[serde(rename_all = "camelCase")]
pub struct ReportData {
    pub cargo_updates: Option<HashMap<String, [Version; 2]>>,
    pub cargo_updates_git: Option<HashMap<String, [ObjectId; 2]>>,
    pub cron_apt: bool,
    pub diskspace_total: u64,
    pub diskspace_free: u64,
    pub inodes_total: u64,
    pub inodes_free: u64,
    pub needrestart: Option<u8>,
    pub oldconffiles: HashMap<String, bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum CargoUpdateCheckError {
    #[error(transparent)] GitHash(#[from] gix_hash::decode::Error),
    #[error(transparent)] SemVer(#[from] semver::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error("`cargo install-update` listed multiple packages with the same name")]
    DuplicatePackage,
    #[error("no table header in `cargo install-update` output")]
    MissingTableHeader,
    #[error("failed to parse “Needs update” column")]
    NeedsUpdate,
    #[error("failed to split table row")]
    SplitAtWidth,
    #[error("missing “v” prefix on version")]
    VersionPrefix,
}

pub async fn check_cargo_updates() -> Result<Option<(HashMap<String, [Version; 2]>, HashMap<String, [ObjectId; 2]>)>, CargoUpdateCheckError> {
    fn split_at_width(s: &str, width: usize) -> Result<[&str; 2], CargoUpdateCheckError> {
        let mut idx = s.ceil_char_boundary(width);
        Ok(loop {
            match s[..idx].width().cmp(&width) {
                Less => {
                    if idx >= s.len() { return Err(CargoUpdateCheckError::SplitAtWidth) }
                    idx = s.ceil_char_boundary(idx + 1);
                }
                Equal => {
                    let (a, b) = s.split_at(idx);
                    break [a, b]
                }
                Greater => idx = s.floor_char_boundary(idx.checked_sub(1).ok_or(CargoUpdateCheckError::SplitAtWidth)?),
            }
        })
    }

    let output = match Command::new("sudo").arg("-n").arg("-u").arg("fenhl").arg("/home/fenhl/.cargo/bin/cargo").arg("install-update").arg("--list").arg("--git").release_create_no_window().check("cargo install-update").await {
        Ok(output) => output,
        Err(wheel::Error::Io { inner, .. }) if inner.kind() == io::ErrorKind::NotFound => return Ok(None), // `cargo` not in PATH
        Err(wheel::Error::CommandExit { output, .. }) if output.status.code().is_some_and(|code| code == 101) => return Ok(None), // `cargo install-update` not installed
        Err(e) => return Err(e.into()),
    };
    let mut lines = output.stdout.lines();
    let (package_width, installed_width, latest_width) = loop {
        let line = lines.next().ok_or(CargoUpdateCheckError::MissingTableHeader)?.at_command("cargo install-update")?;
        if let Some((_, package, installed, latest)) = regex_captures!("^(Package +)(Installed +)(Latest +)Needs update$", &line) {
            break (package.width(), installed.width(), latest.width())
        }
    };
    let mut cargo_updates = HashMap::default();
    for line in &mut lines {
        let line = line.at_command("cargo install-update")?;
        if line.is_empty() { break }
        let [package, rest] = split_at_width(&line, package_width)?;
        let [installed, rest] = split_at_width(rest, installed_width)?;
        let [latest, needs_update] = split_at_width(rest, latest_width)?;
        let package = package.trim_end();
        let installed = installed.trim_end().strip_prefix('v').ok_or(CargoUpdateCheckError::VersionPrefix)?.parse()?;
        let mut latest = latest.trim_end().strip_prefix('v').ok_or(CargoUpdateCheckError::VersionPrefix)?;
        if let Some((prefix, _)) = latest.split_once(' ') {
            latest = prefix;
        }
        let latest = latest.parse()?;
        let needs_update = match needs_update {
            "No" => false,
            "Yes" => true,
            _ => return Err(CargoUpdateCheckError::NeedsUpdate),
        };
        if needs_update {
            if cargo_updates.insert(package.to_owned(), [installed, latest]).is_some() { return Err(CargoUpdateCheckError::DuplicatePackage) }
        }
    }
    let (package_width, installed_width, latest_width) = loop {
        let line = lines.next().ok_or(CargoUpdateCheckError::MissingTableHeader)?.at_command("cargo install-update")?;
        if let Some((_, package, installed, latest)) = regex_captures!("^(Package +)(Installed +)(Latest +)Needs update$", &line) {
            break (package.width(), installed.width(), latest.width())
        }
    };
    let mut cargo_updates_git = HashMap::default();
    for line in lines {
        let line = line.at_command("cargo install-update")?;
        if line.is_empty() { break }
        let [package, rest] = split_at_width(&line, package_width)?;
        let [installed, rest] = split_at_width(rest, installed_width)?;
        let [latest, needs_update] = split_at_width(rest, latest_width)?;
        let package = package.trim_end();
        let installed = installed.trim_end().parse()?;
        let latest = latest.trim_end().parse()?;
        let needs_update = match needs_update {
            "No" => false,
            "Yes" => true,
            _ => return Err(CargoUpdateCheckError::NeedsUpdate),
        };
        if needs_update {
            if cargo_updates_git.insert(package.to_owned(), [installed, latest]).is_some() { return Err(CargoUpdateCheckError::DuplicatePackage) }
        }
    }
    Ok(Some((cargo_updates, cargo_updates_git)))
}
