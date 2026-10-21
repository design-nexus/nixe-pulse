//! Real device enumeration from the native call engine.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AvailableDevices {
    pub audio_inputs: Vec<DeviceInfo>,
    pub audio_outputs: Vec<DeviceInfo>,
    pub video_inputs: Vec<DeviceInfo>,
}
impl AvailableDevices {
    pub fn detect() -> Self {
        let data = crate::engine::Engine::devices().unwrap_or_default();
        let list = |key: &str| {
            data[key]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, item)| DeviceInfo {
                    id: item["metadata"].as_str().unwrap_or("").into(),
                    name: item["name"].as_str().unwrap_or("Device").into(),
                    is_default: index == 0,
                })
                .collect()
        };
        Self { audio_inputs: list("microphone"), audio_outputs: list("speaker"), video_inputs: list("camera") }
    }
}
