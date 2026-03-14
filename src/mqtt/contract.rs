use serde::{Deserialize, Serialize};

use crate::board::BoardProfile;
use crate::config::types::{MqttConfig, ResourceConfig};
use crate::error::AppError;
use crate::gpio::ResourceConfigSnapshot;
use crate::mqtt::{MqttLastWill, MqttManager, MqttMessage, QoS};

#[derive(Debug, Clone)]
pub struct MqttTopics {
    base_topic: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceAvailabilityPayload {
    pub state: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfoPayload {
    pub device_id: String,
    pub client_id: String,
    pub board_id: String,
    pub board_name: String,
    pub firmware_version: String,
    pub topic_root: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigOperationResultPayload {
    pub request_id: Option<String>,
    pub command: &'static str,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum ContractAction {
    GetConfig {
        request_id: Option<String>,
    },
    ValidateResources {
        request_id: Option<String>,
        resources: ResourceConfig,
    },
    SetResources {
        request_id: Option<String>,
        resources: ResourceConfig,
    },
    PublishResult(ConfigOperationResultPayload),
}

#[derive(Debug, Deserialize)]
struct RequestEnvelope {
    request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResourceCommandEnvelope {
    request_id: Option<String>,
    resources: ResourceConfig,
}

#[derive(Debug, Clone)]
pub struct MqttContract {
    topics: MqttTopics,
    info: DeviceInfoPayload,
}

impl MqttTopics {
    pub fn new(base_topic: impl Into<String>) -> Self {
        Self {
            base_topic: base_topic.into(),
        }
    }

    pub fn availability(&self) -> String {
        format!("{}/status/availability", self.base_topic)
    }

    pub fn info(&self) -> String {
        format!("{}/status/info", self.base_topic)
    }

    pub fn board_config(&self) -> String {
        format!("{}/config/board", self.base_topic)
    }

    pub fn resources_config(&self) -> String {
        format!("{}/config/resources", self.base_topic)
    }

    pub fn config_result(&self) -> String {
        format!("{}/evt/config_result", self.base_topic)
    }

    pub fn get_config_command(&self) -> String {
        format!("{}/cmd/get_config", self.base_topic)
    }

    pub fn validate_resources_command(&self) -> String {
        format!("{}/cmd/validate_resources", self.base_topic)
    }

    pub fn set_resources_command(&self) -> String {
        format!("{}/cmd/set_resources", self.base_topic)
    }

    pub fn command_wildcard(&self) -> String {
        format!("{}/cmd/#", self.base_topic)
    }

    pub fn module_command(&self, module_id: &str) -> String {
        format!("{}/mod/{module_id}/cmd", self.base_topic)
    }

    pub fn module_state(&self, module_id: &str) -> String {
        format!("{}/mod/{module_id}/state", self.base_topic)
    }

    pub fn module_command_wildcard(&self) -> String {
        format!("{}/mod/+/cmd", self.base_topic)
    }

    pub fn parse_module_command_topic(&self, topic: &str) -> Option<String> {
        let prefix = format!("{}/mod/", self.base_topic);
        let suffix = "/cmd";

        if !topic.starts_with(&prefix) || !topic.ends_with(suffix) {
            return None;
        }

        let module_id = &topic[prefix.len()..topic.len() - suffix.len()];
        if module_id.is_empty() || module_id.contains('/') {
            None
        } else {
            Some(module_id.to_string())
        }
    }
}

impl MqttContract {
    pub fn new(mqtt: &MqttConfig, board: &'static BoardProfile) -> Self {
        let topics = MqttTopics::new(mqtt.base_topic.clone());
        let info = DeviceInfoPayload {
            device_id: mqtt.client_id.clone(),
            client_id: mqtt.client_id.clone(),
            board_id: board.id.to_string(),
            board_name: board.name.to_string(),
            firmware_version: env!("CARGO_PKG_VERSION").to_string(),
            topic_root: mqtt.base_topic.clone(),
        };

        Self { topics, info }
    }

    pub fn topics(&self) -> &MqttTopics {
        &self.topics
    }

    pub fn last_will(&self) -> Result<MqttLastWill, AppError> {
        Ok(MqttLastWill {
            topic: self.topics.availability(),
            payload: serde_json::to_vec(&DeviceAvailabilityPayload { state: "offline" })?,
            qos: QoS::AtLeastOnce,
            retain: true,
        })
    }

    pub fn publish_birth(
        &self,
        mqtt: &mut MqttManager,
        resources: &ResourceConfigSnapshot,
    ) -> Result<(), AppError> {
        mqtt.publish_json(
            &self.topics.availability(),
            &DeviceAvailabilityPayload { state: "online" },
            QoS::AtLeastOnce,
            true,
        )?;
        mqtt.publish_json(&self.topics.info(), &self.info, QoS::AtLeastOnce, true)?;
        mqtt.publish_json(
            &self.topics.board_config(),
            &resources.board,
            QoS::AtLeastOnce,
            true,
        )?;
        mqtt.publish_json(
            &self.topics.resources_config(),
            resources,
            QoS::AtLeastOnce,
            true,
        )?;

        Ok(())
    }

    pub fn publish_resources_snapshot(
        &self,
        mqtt: &mut MqttManager,
        resources: &ResourceConfigSnapshot,
    ) -> Result<(), AppError> {
        mqtt.publish_json(
            &self.topics.resources_config(),
            resources,
            QoS::AtLeastOnce,
            true,
        )?;

        Ok(())
    }

    pub fn publish_config_result(
        &self,
        mqtt: &mut MqttManager,
        payload: &ConfigOperationResultPayload,
    ) -> Result<(), AppError> {
        mqtt.publish_json(
            &self.topics.config_result(),
            payload,
            QoS::AtLeastOnce,
            false,
        )?;
        Ok(())
    }

    pub fn parse_action(&self, message: &MqttMessage) -> ContractAction {
        let payload = message.payload.as_slice();
        let topic = message.topic.as_str();

        if topic == self.topics.get_config_command() {
            return match decode_request(payload) {
                Ok(request_id) => ContractAction::GetConfig { request_id },
                Err(error) => ContractAction::PublishResult(Self::error_result(
                    "get_config",
                    None,
                    format!("Invalid get_config request: {error}"),
                )),
            };
        }

        if topic == self.topics.validate_resources_command() {
            return match serde_json::from_slice::<ResourceCommandEnvelope>(payload) {
                Ok(request) => ContractAction::ValidateResources {
                    request_id: request.request_id,
                    resources: request.resources,
                },
                Err(error) => ContractAction::PublishResult(Self::error_result(
                    "validate_resources",
                    None,
                    format!("Invalid validate_resources request: {error}"),
                )),
            };
        }

        if topic == self.topics.set_resources_command() {
            return match serde_json::from_slice::<ResourceCommandEnvelope>(payload) {
                Ok(request) => ContractAction::SetResources {
                    request_id: request.request_id,
                    resources: request.resources,
                },
                Err(error) => ContractAction::PublishResult(Self::error_result(
                    "set_resources",
                    None,
                    format!("Invalid set_resources request: {error}"),
                )),
            };
        }

        ContractAction::PublishResult(Self::error_result(
            "unknown",
            None,
            format!("Unsupported MQTT command topic '{}'", message.topic),
        ))
    }

    pub fn ok_result(
        command: &'static str,
        request_id: Option<String>,
        message: impl Into<String>,
    ) -> ConfigOperationResultPayload {
        ConfigOperationResultPayload {
            request_id,
            command,
            ok: true,
            message: message.into(),
        }
    }

    pub fn error_result(
        command: &'static str,
        request_id: Option<String>,
        message: impl Into<String>,
    ) -> ConfigOperationResultPayload {
        ConfigOperationResultPayload {
            request_id,
            command,
            ok: false,
            message: message.into(),
        }
    }
}

fn decode_request(payload: &[u8]) -> Result<Option<String>, AppError> {
    if payload.is_empty() {
        return Ok(None);
    }

    Ok(serde_json::from_slice::<RequestEnvelope>(payload)?.request_id)
}
