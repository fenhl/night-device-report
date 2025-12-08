#![allow(unused_crate_dependencies)] // combined lib/bin crate

use {
    std::{
        env,
        ffi::OsString,
        time::Duration,
    },
    serde::Serialize,
    tokio::process::Command,
    wheel::traits::IoResultExt as _,
    night_device_report::{
        Config,
        Error,
        ReportData,
    },
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CronReport {
    key: String,
    status: Option<i32>,
}

#[derive(clap::Parser)]
#[clap(version)]
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
        let data = ReportData::new(&config).await?;
        client.post(&format!("https://night.fenhl.net/dev/{}/device-report", config.hostname()?))
            .bearer_auth(&config.device_key)
            .json(&data)
            .send().await?
            .error_for_status()?;
    }
    Ok(())
}
