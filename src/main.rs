#![allow(unused_crate_dependencies)] // combined lib/bin crate

use {
    std::{
        env,
        ffi::OsString,
        num::ParseIntError,
        path::Path,
        process::Stdio,
        string,
        time::Duration,
    },
    futures::stream::TryStreamExt as _,
    gethostname::gethostname,
    serde::{
        Deserialize,
        Serialize,
    },
    systemstat::{
        Platform as _,
        System,
    },
    tokio::{
        io::{
            AsyncBufReadExt as _,
            BufReader,
        },
        process::Command,
    },
    tokio_stream::wrappers::LinesStream,
    wheel::{
        fs::{
            self,
            File,
        },
        traits::IoResultExt as _,
    },
    night_device_report::ReportData,
};
#[cfg(feature = "async-proto")] use async_proto as _; // only used in lib target
#[cfg(unix)] use std::path::PathBuf;
#[cfg(windows)] use directories::ProjectDirs;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] CargoUpdateCheck(#[from] night_device_report::CargoUpdateCheckError),
    #[error(transparent)] Config(#[from] ConfigError),
    #[error(transparent)] ParseInt(#[from] ParseIntError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] TryFromInt(#[from] std::num::TryFromIntError),
    #[error(transparent)] Utf8(#[from] string::FromUtf8Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error("non-UTF-8 string")]
    OsString(OsString),
}

impl From<OsString> for Error {
    fn from(value: OsString) -> Self {
        Self::OsString(value)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    device_key: String,
    hostname: Option<String>,
    /// Whether I have root access on this device.
    /// If `true`, night-device-report assumes it is running as `root`.
    /// If `false`, night-device-report skips checks for system updates which should be handled by root.
    #[serde(default = "make_true")]
    root: bool,
}

#[derive(Debug, thiserror::Error)]
enum ConfigError {
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

impl Config {
    async fn load() -> Result<Self, ConfigError> {
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

    fn hostname(&self) -> Result<String, OsString> {
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CronReport {
    key: String,
    status: Option<i32>,
}

fn make_true() -> bool { true }

#[derive(clap::Parser)]
struct Args {
    #[clap(requires = "cmd")]
    cronjob: Option<String>,
    cmd: Option<OsString>,
    args: Vec<OsString>,
}

#[wheel::main]
async fn main(args: Args) -> Result<(), Error> {
    let config = Config::load().await?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("night-device-report/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(600))
        .http2_prior_knowledge()
        .use_rustls_tls()
        .https_only(true)
        .build()?;
    if let (Some(cronjob), Some(cmd)) = (args.cronjob, args.cmd) {
        let status = Command::new(&cmd).args(args.args).status().await.at_command(cmd.to_string_lossy().into_owned())?;
        let data = CronReport {
            key: config.device_key.clone(),
            status: status.code(),
        };
        client.post(&format!("https://night.fenhl.net/device-report/{}/{}", config.hostname()?, cronjob))
            .json(&data)
            .send().await?
            .error_for_status()?;
    } else {
        let (cargo_updates, cargo_updates_git) = if let Some((cargo_updates, cargo_updates_git)) = night_device_report::check_cargo_updates().await? {
            (Some(cargo_updates), Some(cargo_updates_git))
        } else {
            (None, None)
        };
        let fs = System::new().mount_at("/").at("/")?;
        let data = ReportData {
            cron_apt: config.root && if let os_info::Type::NixOS = os_info::get().os_type() {
                false // updates are configured to be installed automatically
            } else {
                // not NixOS, assume Debian
                let mut cron_apt = false;
                let syslogs = vec![Path::new("/var/log/syslog"), Path::new("/var/log/syslog.1")];
                'cron_apt: for log_path in syslogs {
                    if log_path.exists() {
                        let log_f = BufReader::new(File::open(log_path).await?);
                        for line in LinesStream::new(log_f.lines()).try_collect::<Vec<_>>().await.at(log_path)?.into_iter().rev() {
                            if line.contains("cron-apt: Download complete and in download only mode") {
                                cron_apt = true;
                                break 'cron_apt
                            } else if line.contains("cron-apt: 0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.") {
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
            needrestart: if config.root {
                match os_info::get().os_type() { // emulate NEEDRESTART-KSTA codes
                    os_info::Type::Macos => Some(1), // update workflow includes reboot
                    os_info::Type::NixOS => match Command::new("nixos-needsreboot").status().await.at_command("nixos-needsreboot")?.code() {
                        Some(0) => Some(1), // no reboot needed
                        Some(2) => Some(2), // reboot needed //TODO use 3 if it's specifically for a new kernel version (check stderr)
                        code => {
                            if let Some(code) = code {
                                eprintln!("nixos-needsreboot exited with status code {code}");
                            } else {
                                eprintln!("nixos-needsreboot exited with no status code");
                            }
                            Some(0) // unknown status
                        }
                    },
                    _ => String::from_utf8(Command::new("/usr/sbin/needrestart").arg("-b").stderr(Stdio::null()).output().await.at_command("needrestart")?.stdout)?.lines()
                        .find_map(|line| line.strip_prefix("NEEDRESTART-KSTA: "))
                        .map(|line| line.parse())
                        .transpose()?,
                }
            } else { None },
            oldconffiles: {
                ["fenhl", "pi"].into_iter()
                    .map(|username| (username.into(), Path::new("/home").join(username).join("oldconffiles").exists()))
                    .collect()
            },
            cargo_updates, cargo_updates_git,
        };
        client.post(&format!("https://night.fenhl.net/dev/{}/device-report", config.hostname()?))
            .bearer_auth(&config.device_key)
            .json(&data)
            .send().await?
            .error_for_status()?;
    }
    Ok(())
}
