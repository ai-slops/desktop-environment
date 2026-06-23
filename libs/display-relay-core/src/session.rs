use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureTarget {
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayConfig {
    pub target: CaptureTarget,
    pub mirror_fullscreen: bool,
    pub capture_timeout_ms: u32,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            target: CaptureTarget { display_name: String::new() },
            mirror_fullscreen: false,
            capture_timeout_ms: 16,
        }
    }
}
