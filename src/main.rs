#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes,unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::HashMap,
        env,
        ffi::OsString,
        fmt,
        fs::File,
        io::{
            self,
            BufRead,
            BufReader,
        },
        num::ParseIntError,
        path::Path,
        process::{
            Command,
            Stdio,
        },
        string,
        time::Duration,
    },
    derive_more::From,
    gethostname::gethostname,
    serde::{
        Deserialize,
        Serialize,
    },
    structopt::StructOpt,
    systemstat::{
        Platform,
        System,
    },
};

#[derive(Debug, From)]
enum Error {
    Io(io::Error),
    Json(serde_json::Error),
    MissingConfig,
    ParseInt(ParseIntError),
    Reqwest(reqwest::Error),
    Utf8(string::FromUtf8Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {}", e),
            Error::Json(e) => write!(f, "JSON error: {}", e),
            Error::MissingConfig => write!(f, "config file not found"),
            Error::ParseInt(e) => e.fmt(f),
            Error::Reqwest(e) => if let Some(url) = e.url() {
                write!(f, "HTTP error at {}: {}", url, e)
            } else {
                write!(f, "HTTP error: {}", e)
            },
            Error::Utf8(e) => e.fmt(f),
        }?;
        write!(f, "\nerror code: {:?}", self)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    device_key: String,
    hostname: Option<String>,
    #[serde(default = "make_true")]
    root: bool,
}

impl Config {
    fn new() -> Result<Config, Error> {
        let dirs = xdg_basedir::get_config_home().into_iter().chain(xdg_basedir::get_config_dirs());
        let file = dirs.filter_map(|cfg_dir| File::open(cfg_dir.join("fenhl/night.json")).ok())
            .next().ok_or(Error::MissingConfig)?;
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
    inodes_total: usize,
    inodes_free: usize,
    needrestart: Option<u8>,
    oldconffiles: HashMap<String, bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CronReport {
    key: String,
    status: Option<i32>,
}

fn make_true() -> bool { true }

/// stand-in for `Option::transpose` since it's not stable on Rust 1.32.0
fn transpose<T, E>(o: Option<Result<T, E>>) -> Result<Option<T>, E> {
    match o {
        Some(Ok(v)) => Ok(Some(v)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

#[derive(StructOpt)]
struct Args {
    #[structopt(requires = "cmd")]
    cronjob: Option<String>,
    #[structopt(parse(from_os_str))]
    cmd: Option<OsString>,
    #[structopt(parse(from_os_str))]
    args: Vec<OsString>,
}

#[derive(StructOpt)]
struct Cronjob {
}

#[wheel::main]
async fn main(args: Args) -> Result<(), Error> {
    let config = Config::new()?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("night-device-report/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(600))
        //TODO enable http2_prior_knowledge after it's enabled on nightd
        .use_rustls_tls()
        .https_only(true)
        .build()?;
    if let (Some(cronjob), Some(cmd)) = (args.cronjob, args.cmd) {
        let status = Command::new(cmd).args(args.args).status()?;
        let data = CronReport {
            key: config.device_key.clone(),
            status: status.code(),
        };
        client.post(&format!("https://nightd.fenhl.net/device-report/{}/{}", config.hostname(), cronjob))
            .json(&data)
            .send().await?
            .error_for_status()?;
    } else {
        let fs = System::new().mount_at("/")?;
        let data = ReportData {
            key: config.device_key.clone(),
            cron_apt: config.root && {
                let mut cron_apt = false;
                let syslogs = vec![Path::new("/var/log/syslog"), Path::new("/var/log/syslog.1")];
                'cron_apt: for log_path in syslogs {
                    if log_path.exists() {
                        let log_f = BufReader::new(File::open(log_path)?);
                        for line in log_f.lines().filter_map(Result::ok).collect::<Vec<_>>().into_iter().rev() {
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
            inodes_total: fs.files_total,
            inodes_free: fs.files_avail,
            needrestart: if config.root {
                let ksta = String::from_utf8(Command::new("/usr/sbin/needrestart").arg("-b").stderr(Stdio::null()).output()?.stdout)?.lines()
                    .find(|line| line.starts_with("NEEDRESTART-KSTA: "))
                    .map(|line| line["NEEDRESTART-KSTA: ".len()..].parse());
                transpose(ksta)?
            } else { None },
            oldconffiles: {
                vec!["fenhl", "pi"].into_iter()
                    .map(|username| (username.into(), Path::new("/home").join(username).join("oldconffiles").exists()))
                    .collect()
            },
        };
        client.post(&format!("https://nightd.fenhl.net/device-report/{}", config.hostname()))
            .json(&data)
            .send().await?
            .error_for_status()?;
    }
    Ok(())
}
