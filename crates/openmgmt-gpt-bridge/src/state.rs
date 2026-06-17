use crate::config::GptBridgeConfig;
use openmgmt_core::AppService;
use std::sync::Arc;

#[derive(Clone)]
pub struct BridgeState {
    pub config: Arc<GptBridgeConfig>,
    pub service: AppService,
}
