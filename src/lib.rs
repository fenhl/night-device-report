#![allow(unused_crate_dependencies)] // combined lib/bin crate

use {
    std::collections::HashMap,
    serde::{
        Deserialize,
        Serialize,
    },
};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportData {
    pub cron_apt: bool,
    pub diskspace_total: u64,
    pub diskspace_free: u64,
    pub inodes_total: usize,
    pub inodes_free: usize,
    pub needrestart: Option<u8>,
    pub oldconffiles: HashMap<String, bool>,
}
