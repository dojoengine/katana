use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const STATE_FILE: &str = ".tee_deployment_state.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentState {
    /// Block number when contracts were deployed
    pub deployment_block: Option<u64>,
    /// Address of the deployed AMDTeeRegistry
    pub amd_tee_registry_address: Option<String>,
    /// Address of the deployed KatanaTee
    pub katana_tee_address: Option<String>,
}

impl DeploymentState {
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if !Path::new(STATE_FILE).exists() {
            tracing::info!("State file not found, using default state");
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(STATE_FILE)?;
        let state: DeploymentState = serde_json::from_str(&contents)?;
        tracing::info!("Loaded state from {}: {:?}", STATE_FILE, state);
        Ok(state)
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(STATE_FILE, json)?;
        tracing::info!("Saved state to {}: {:?}", STATE_FILE, self);
        Ok(())
    }

    pub fn get_amd_tee_registry_address(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.amd_tee_registry_address
            .clone()
            .ok_or_else(|| "AMDTeeRegistry address not found. Run init first.".into())
    }

    pub fn get_katana_tee_address(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.katana_tee_address
            .clone()
            .ok_or_else(|| "KatanaTee address not found. Run init first.".into())
    }
}
