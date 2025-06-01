#![allow(unused_crate_dependencies)] // combined lib/bin crate

use {
    std::collections::HashMap,
    serde::{
        Deserialize,
        Serialize,
    },
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "async-proto", derive(async_proto::Protocol))]
#[serde(rename_all = "camelCase")]
pub struct ReportData {
    pub cron_apt: bool,
    pub diskspace_total: u64,
    pub diskspace_free: u64,
    pub inodes_total: u64,
    pub inodes_free: u64,
    pub needrestart: Option<u8>,
    pub oldconffiles: HashMap<String, bool>,
}
