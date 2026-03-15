use esp_idf_svc::mqtt::client::QoS;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::modules::{DriverBinaryState, ModuleManager};
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;

use super::traits::DeviceController;

#[derive(Debug, Serialize)]
struct SwitchStatePayload {
    state: &'static str,
}

#[derive(Debug, Deserialize)]
struct SwitchCommandPayload {
    state: String,
}

pub struct SwitchController {
    id: String,
    driver_module_id: String,
}

impl SwitchController {
    pub fn new(id: String, driver_module_id: String) -> Self {
        Self {
            id,
            driver_module_id,
        }
    }
}

impl DeviceController for SwitchController {
    fn id(&self) -> &str {
        &self.id
    }

    fn driver_module_id(&self) -> &str {
        &self.driver_module_id
    }

    fn publish_state(
        &self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
        modules: &ModuleManager,
    ) -> Result<(), AppError> {
        let state = modules
            .binary_state(self.driver_module_id())
            .ok_or_else(|| {
                AppError::Message(format!(
                    "Driver module '{}' not available for device '{}'",
                    self.driver_module_id(),
                    self.id()
                ))
            })?;
        let payload = SwitchStatePayload {
            state: match state {
                DriverBinaryState::On => "ON",
                DriverBinaryState::Off => "OFF",
            },
        };

        mqtt.publish_json(
            &topics.device_state(self.id()),
            &payload,
            QoS::AtLeastOnce,
            true,
        )?;

        Ok(())
    }

    fn handle_command(
        &self,
        payload: &[u8],
        modules: &mut ModuleManager,
    ) -> Result<Vec<String>, AppError> {
        let command = parse_switch_command(payload)?;
        modules.execute_command(self.driver_module_id(), &command)
    }
}

fn parse_switch_command(payload: &[u8]) -> Result<String, AppError> {
    if payload.is_empty() {
        return Err(AppError::Message(
            "Switch command payload cannot be empty".into(),
        ));
    }

    if payload[0] == b'{' {
        let payload = serde_json::from_slice::<SwitchCommandPayload>(payload)?;
        return normalize_switch_command(&payload.state);
    }

    let payload = std::str::from_utf8(payload)
        .map_err(|error| AppError::Message(format!("Invalid switch command UTF-8: {error}")))?;
    normalize_switch_command(payload)
}

fn normalize_switch_command(value: &str) -> Result<String, AppError> {
    let normalized = value.trim().to_uppercase();

    match normalized.as_str() {
        "ON" | "OFF" | "TOGGLE" => Ok(normalized),
        _ => Err(AppError::Message(format!(
            "Unsupported switch command '{}'",
            value
        ))),
    }
}
