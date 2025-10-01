use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct ViewTransferConfig {
    pub timeout_duration: Duration,
}
