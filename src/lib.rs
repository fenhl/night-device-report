use {
    std::{
        cmp::Ordering::*,
        ffi::OsString,
        io::prelude::*,
    },
    clap as _, // only used in bin target
    gethostname::gethostname,
    gix_hash::ObjectId,
    itertools::Itertools as _,
    lazy_regex::regex_captures,
    std::collections::HashMap,
    semver::Version,
    serde::{
        Deserialize,
        Serialize,
    },
    systemstat::{
        Platform as _,
        System,
    },
    tokio::{
        io,
        process::Command,
    },
    unicode_width::UnicodeWidthStr as _,
    wheel::{
        fs,
        traits::{
            AsyncCommandOutputExt as _,
            IoResultExt as _,
        },
    },
};
#[cfg(unix)] use {
    std::{
        iter,
        path::{
            Path,
            PathBuf,
        },
        process::Stdio,
        str::FromStr as _,
    },
    futures::stream::TryStreamExt as _,
    lazy_regex::regex_is_match,
    tokio::io::{
        AsyncBufReadExt as _,
        BufReader,
    },
    tokio_stream::wrappers::LinesStream,
    wheel::fs::File,
};
#[cfg(windows)] use {
    directories::ProjectDirs,
    wheel::traits::CommandExt as _,
};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[cfg(unix)]
    #[error("config file not found")]
    Missing {
        config_home: Option<PathBuf>,
        config_dirs: Vec<PathBuf>,
    },
    #[cfg(windows)]
    #[error("failed to find project folder")]
    ProjectDirs,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)] CargoUpdateCheck(#[from] CargoUpdateCheckError),
    #[error(transparent)] Config(#[from] ConfigError),
    #[error(transparent)] ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] TryFromInt(#[from] std::num::TryFromIntError),
    #[error(transparent)] Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error("non-UTF-8 string")]
    OsString(OsString),
}

impl From<OsString> for Error {
    fn from(value: OsString) -> Self {
        Self::OsString(value)
    }
}

#[cfg(windows)] fn make_c() -> Vec<String> { vec![format!("C:\\")] }
fn make_true() -> bool { true }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub device_key: String,
    #[cfg(windows)]
    #[serde(default = "make_c")]
    pub file_systems: Vec<String>,
    pub hostname: Option<String>,
    /// Whether I have root access on this device.
    /// If `true`, night-device-report assumes it is running as `root`.
    /// If `false`, night-device-report skips checks for system updates which should be handled by root.
    #[serde(default = "make_true")]
    pub root: bool,
}

impl Config {
    pub async fn load() -> Result<Self, ConfigError> {
        #[cfg(unix)] {
            let base_dirs = xdg::BaseDirectories::new();
            if let Some(path) = base_dirs.find_config_file("fenhl/night.json") {
                Ok(fs::read_json(path).await?)
            } else {
                Err(ConfigError::Missing {
                    config_home: base_dirs.get_config_home(),
                    config_dirs: base_dirs.get_config_dirs(),
                })
            }
        }
        #[cfg(windows)] {
            let path = ProjectDirs::from("net", "Fenhl", "Night").ok_or(ConfigError::ProjectDirs)?.config_dir().join("config.json");
            Ok(fs::read_json(path).await?)
        }
    }

    pub fn hostname(&self) -> Result<String, OsString> {
        Ok(if let Some(ref hostname) = self.hostname {
            hostname.clone()
        } else {
            let full_hostname = gethostname().into_string()?;
            if let Some((prefix, _)) = full_hostname.split_once('.') {
                prefix.to_owned()
            } else {
                full_hostname
            }
        })
    }
}

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
    pub os_version: Option<os_info::Version>,
}

impl ReportData {
    pub async fn new(config: &Config) -> Result<Self, Error> {
        let (cargo_updates, cargo_updates_git) = if let Some((cargo_updates, cargo_updates_git)) = check_cargo_updates(config.root).await? {
            if !cargo_updates.is_empty() || !cargo_updates_git.is_empty() {
                let command = {
                    #[cfg(unix)] {
                        let mut cmd;
                        if config.root {
                            cmd = Command::new("sudo");
                            cmd.arg("-n");
                            cmd.arg("-u");
                            cmd.arg("fenhl");
                            #[cfg(target_os = "macos")] { cmd.arg("/Users/fenhl/.cargo/bin/cargo"); }
                            #[cfg(not(target_os = "macos"))] { cmd.arg("/home/fenhl/.cargo/bin/cargo"); }
                        } else {
                            #[cfg(target_os = "macos")] { cmd = Command::new("/Users/fenhl/.cargo/bin/cargo"); }
                            #[cfg(not(target_os = "macos"))] { cmd = Command::new("/home/fenhl/.cargo/bin/cargo"); }
                        }
                        cmd.arg("install-update");
                        cmd.arg("--all");
                        cmd.arg("--git");
                        cmd
                    }
                    #[cfg(windows)] {
                        let mut cmd = Command::new("cargo");
                        cmd.arg("install-update");
                        cmd.arg("--all");
                        cmd.arg("--git");
                        cmd.release_create_no_window();
                        cmd
                    }
                };
                if command.check("cargo install-update").await.is_ok() {
                    (Some(HashMap::default()), Some(HashMap::default()))
                } else {
                    (Some(cargo_updates), Some(cargo_updates_git))
                }
            } else {
                (Some(cargo_updates), Some(cargo_updates_git))
            }
        } else {
            (None, None)
        };
        let os_info = os_info::get();
        #[cfg(unix)] {
            let fs = System::new().mount_at("/").at("/")?;
            //TODO if low on disk space, run cargo sweep (`cargo sweep -ir` on non-NixOS, need to determine toolchains to keep on NixOS)
            Ok(Self {
                cron_apt: config.root && if let os_info::Type::NixOS = os_info.os_type() {
                    false // updates are configured to be installed automatically, TODO verify nixos-upgrade.service exited successfully
                } else {
                    // not NixOS, assume Debian
                    let mut cron_apt = true;
                    let syslogs = vec![Path::new("/var/log/syslog"), Path::new("/var/log/syslog.1")];
                    'cron_apt: for log_path in syslogs {
                        if log_path.exists() {
                            let log_f = BufReader::new(File::open(log_path).await?);
                            for line in LinesStream::new(log_f.lines()).try_collect::<Vec<_>>().await.at(log_path)?.into_iter().rev() {
                                if line.contains("cron-apt: Download complete and in download only mode") {
                                    break 'cron_apt
                                } else if line.contains("cron-apt: 0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.") {
                                    cron_apt = false;
                                    break 'cron_apt
                                }
                            }
                        }
                    }
                    cron_apt
                },
                diskspace_total: fs.total.as_u64(),
                diskspace_free: fs.avail.as_u64(),
                inodes_total: fs.files_total.try_into()?,
                inodes_free: fs.files_avail.try_into()?,
                needrestart: match os_info.os_type() { // emulate NEEDRESTART-KSTA codes
                    os_info::Type::Macos => Some(1), // update workflow includes reboot
                    os_info::Type::NixOS => if config.root {
                        let output = Command::new("nixos-needsreboot").output().await.at_command("nixos-needsreboot")?;
                        match output.status.code() {
                            Some(0) => Some(1), // no reboot needed
                            Some(2) => Some(2), // reboot needed //TODO use 3 if it's specifically for a new kernel version (check stderr)
                            Some(1) if regex_is_match!("nixos-needsreboot: I/O error at /nix/store/.+/lib/modules: No such file or directory \\(os error 2\\)", &String::from_utf8_lossy(&output.stderr)) => Some(3), // NixOS seems to delete old kernel modules after upgrade
                            code => {
                                if let Some(code) = code {
                                    eprintln!("nixos-needsreboot exited with status code {code}");
                                } else {
                                    eprintln!("nixos-needsreboot exited with no status code");
                                }
                                Some(0) // unknown status
                            }
                        }
                    } else {
                        None
                    },
                    _ => if config.root {
                        String::from_utf8(Command::new("/usr/sbin/needrestart").arg("-b").stderr(Stdio::null()).output().await.at_command("needrestart")?.stdout)?.lines()
                            .find_map(|line| line.strip_prefix("NEEDRESTART-KSTA: "))
                            .map(|line| line.parse())
                            .transpose()?
                    } else {
                        None
                    },
                },
                oldconffiles: {
                    ["fenhl", "pi"].into_iter()
                        .map(|username| (username.into(), Path::new("/home").join(username).join("oldconffiles").exists()))
                        .collect()
                },
                os_version: Some(if let os_info::Type::Debian = os_info.os_type() {
                    // os_info only reports major version, get more accurate version info from file
                    let [major, minor, patch] = fs::read_to_string("/etc/debian_version").await?.split('.').map(u64::from_str).chain(iter::repeat(Ok(0))).next_array().expect("iter::repeat produces an infinite iterator");
                    os_info::Version::Semantic(major?, minor?, patch?)
                } else {
                    os_info.version().clone()
                }),
                cargo_updates, cargo_updates_git,
            })
        }
        #[cfg(windows)] {
            let sys = System::new();
            let fs = config.file_systems.iter()
                .map(|vol| sys.mount_at(vol).at(vol))
                .process_results(|vols| vols.min_by(|fs1, fs2| (fs1.avail.as_u64() as f64 / fs1.total.as_u64() as f64).total_cmp(&(fs2.avail.as_u64() as f64 / fs2.total.as_u64() as f64))))?
                .expect("should be nonempty if there was no error");
            Ok(Self {
                cron_apt: true, // see night-windows-service crate in private night repo for a way to actually check for updates
                diskspace_total: fs.total.as_u64(),
                diskspace_free: fs.avail.as_u64(),
                inodes_total: fs.files_total.try_into()?,
                inodes_free: fs.files_avail.try_into()?,
                needrestart: Some(2), //TODO see cron_apt field; Some(1) for no reboot needed, Some(2) for reboot needed
                oldconffiles: HashMap::default(),
                os_version: Some(os_info.version().clone()),
                cargo_updates, cargo_updates_git,
            })
        }
    }
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

pub async fn check_cargo_updates(#[cfg_attr(windows, allow(unused))] root: bool) -> Result<Option<(HashMap<String, [Version; 2]>, HashMap<String, [ObjectId; 2]>)>, CargoUpdateCheckError> {
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

    let command = {
        #[cfg(unix)] {
            let mut cmd;
            if root {
                cmd = Command::new("sudo");
                cmd.arg("-n");
                cmd.arg("-u");
                cmd.arg("fenhl");
                #[cfg(target_os = "macos")] { cmd.arg("/Users/fenhl/.cargo/bin/cargo"); }
                #[cfg(not(target_os = "macos"))] { cmd.arg("/home/fenhl/.cargo/bin/cargo"); }
            } else {
                #[cfg(target_os = "macos")] { cmd = Command::new("/Users/fenhl/.cargo/bin/cargo"); }
                #[cfg(not(target_os = "macos"))] { cmd = Command::new("/home/fenhl/.cargo/bin/cargo"); }
            }
            cmd.arg("install-update");
            cmd.arg("--list");
            cmd.arg("--git");
            cmd
        }
        #[cfg(windows)] {
            let mut cmd = Command::new("cargo");
            cmd.arg("install-update");
            cmd.arg("--list");
            cmd.arg("--git");
            cmd.release_create_no_window();
            cmd
        }
    };
    let output = match command.check("cargo install-update").await {
        Ok(output) => output,
        Err(wheel::Error::Io { inner, .. }) if inner.kind() == io::ErrorKind::NotFound => return Ok(None), // `cargo` not in PATH
        Err(wheel::Error::CommandExit { output, .. }) if output.status.code().is_some_and(|code| code == 101) => return Ok(None), // `cargo install-update` not installed
        Err(e) => return Err(e.into()),
    };
    let mut lines = BufRead::lines(&*output.stdout);
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
