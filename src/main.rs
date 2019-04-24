use std::{
    collections::HashMap,
    fs::File,
    io::{
        self,
        BufRead,
        BufReader
    },
    num::ParseIntError,
    path::Path,
    process::{
        Command,
        Stdio
    },
    string,
    time::Duration
};
use gethostname::gethostname;
use serde_derive::{
    Deserialize,
    Serialize
};
use systemstat::{
    Platform,
    System
};
use wrapped_enum::wrapped_enum;

#[derive(Debug)]
enum OtherError {
    MissingConfig
}

wrapped_enum! {
    #[derive(Debug)]
    enum Error {
        Io(io::Error),
        Json(serde_json::Error),
        Other(OtherError),
        ParseInt(ParseIntError),
        Reqwest(reqwest::Error),
        Utf8(string::FromUtf8Error)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    device_key: String,
    hostname: Option<String>
}

impl Config {
    fn new() -> Result<Config, Error> {
        let dirs = xdg_basedir::get_config_home().into_iter().chain(xdg_basedir::get_config_dirs());
        let file = dirs.filter_map(|cfg_dir| File::open(cfg_dir.join("fenhl/night.json")).ok())
            .next().ok_or(OtherError::MissingConfig)?;
        Ok(serde_json::from_reader(file)?)
    }

    fn hostname(self) -> String {
        self.hostname.unwrap_or_else(|| gethostname().into_string().expect("hostname is invalid UTF-8").split('.').next().expect("hostname is empty").into())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportData {
    key: String,
    cron_apt: bool,
    diskspace_total: u64,
    diskspace_free: u64,
    needrestart: Option<u8>,
    oldconffiles: HashMap<String, bool>
}

#[cfg(target_pointer_width = "64")]
fn diskspace() -> Result<(u64, u64), Error> {
    let fs = System::new().mount_at("/")?;
    Ok((fs.total.as_usize() as u64, fs.avail.as_usize() as u64))
}

#[cfg(target_pointer_width = "32")]
fn diskspace() -> Result<(u64, u64), Error> {
    Ok((
        String::from_utf8(Command::new("python3").arg("-c").arg("import shutil; print(shutil.disk_usage(\"/\").total)").stdout(Stdio::piped()).output()?.stdout)?.parse()?,
        String::from_utf8(Command::new("python3").arg("-c").arg("import shutil; print(shutil.disk_usage(\"/\").free)").stdout(Stdio::piped()).output()?.stdout)?.parse()?
    ))
}

/// stand-in for `Option::transpose` since it's not stable on Rust 1.32.0
fn transpose<T, E>(o: Option<Result<T, E>>) -> Result<Option<T>, E> {
    match o {
        Some(Ok(v)) => Ok(Some(v)),
        Some(Err(e)) => Err(e),
        None => Ok(None)
    }
}

fn main() -> Result<(), Error> {
    let config = Config::new()?;
    let (diskspace_total, diskspace_free) = diskspace()?;
    let data = ReportData {
        key: config.device_key.clone(),
        cron_apt: {
            let mut cron_apt = false;
            let syslogs = vec![Path::new("/var/log/syslog"), Path::new("/var/log/syslog.1")];
            'cron_apt: for log_path in syslogs {
                if log_path.exists() {
                    let log_f = BufReader::new(File::open(log_path)?);
                    for line in log_f.lines().filter_map(Result::ok).collect::<Vec<_>>().into_iter().rev() {
                        if line.contains("cron-apt: Download complete and in download only mode") {
                            cron_apt = true;
                            break 'cron_apt;
                        } else if line.contains("cron-apt: 0 upgraded, 0 newly installed, 0 to remove and 0 not upgraded.") {
                            break 'cron_apt;
                        }
                    }
                }
            }
            cron_apt
        },
        diskspace_total,
        diskspace_free,
        needrestart: {
            let ksta = String::from_utf8(Command::new("/usr/sbin/needrestart").arg("-b").stderr(Stdio::null()).output()?.stdout)?.lines()
                .find(|line| line.starts_with("NEEDRESTART-KSTA: "))
                .map(|line| line["NEEDRESTART-KSTA: ".len()..].parse());
            transpose(ksta)?
        },
        oldconffiles: {
            vec!["fenhl", "pi"].into_iter()
                .map(|username| (username.into(), Path::new("/home").join(username).join("oldconffiles").exists()))
                .collect()
        }
    };
    reqwest::Client::builder()
        .timeout(Some(Duration::from_secs(600)))
        .build()?
        .post(&format!("https://nightd.fenhl.net/device-report/{}", config.hostname()))
        .json(&data)
        .send()?
        .error_for_status()?;
    Ok(())
}
