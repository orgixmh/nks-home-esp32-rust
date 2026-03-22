use serde::{Deserialize, Deserializer, Serialize};

pub const RESOURCE_CONFIG_VERSION: u32 = 1;

fn default_resource_config_version() -> u32 {
    RESOURCE_CONFIG_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiConfig {
    pub ssid: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub client_id: String,
    pub base_topic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub wifi: WifiConfig,
    pub mqtt: MqttConfig,
    #[serde(default)]
    pub resources: ResourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConfig {
    #[serde(default = "default_resource_config_version")]
    pub version: u32,
    #[serde(default)]
    pub module_instances: Vec<ModuleInstanceConfig>,
    #[serde(default)]
    pub device_instances: Vec<DeviceInstanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInstanceConfig {
    pub id: String,
    #[serde(alias = "module_type", deserialize_with = "deserialize_module_type_id")]
    pub module_type_id: String,
    pub display_name: Option<String>,
    pub bindings: Vec<PinBindingConfig>,
    #[serde(default)]
    pub settings: ModuleSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInstanceConfig {
    pub id: String,
    #[serde(alias = "device_type", deserialize_with = "deserialize_device_type_id")]
    pub device_type_id: String,
    pub display_name: Option<String>,
    pub driver_module_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceUsage {
    Input,
    Output,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinBindingConfig {
    #[serde(alias = "role", deserialize_with = "deserialize_role_id")]
    pub role_id: String,
    pub target: ResourceBindingTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceBindingTarget {
    Gpio { pin: u8 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModuleSettings {
    #[serde(default)]
    pub auto_off_ms: Option<u64>,
    #[serde(default)]
    pub external_on_triggers: Vec<ExternalModuleTriggerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalModuleTriggerConfig {
    pub source_module_id: String,
}

impl DeviceConfig {
    pub fn is_complete(&self) -> bool {
        !self.wifi.ssid.trim().is_empty()
            && !self.mqtt.host.trim().is_empty()
            && self.mqtt.port > 0
            && !self.mqtt.client_id.trim().is_empty()
            && !self.mqtt.base_topic.trim().is_empty()
    }

    pub fn is_demo_seed_config(&self) -> bool {
        let is_demo_ssid = self.wifi.ssid == "eps-rust-test" || self.wifi.ssid == "esp-rust-test";

        is_demo_ssid
            || (self.wifi.password == "asdfg43v34t34f34t3"
                && self.mqtt.host == "10.0.0.1"
                && self.mqtt.port == 1883
                && self.mqtt.username == "testuser"
                && self.mqtt.password == "testpassword"
                && self.mqtt.client_id == "esp32-test-node"
                && self.mqtt.base_topic == "nks/home/test-node")
    }
}

impl PinBindingConfig {
    pub fn pin(&self) -> Result<u8, crate::error::AppError> {
        match self.target {
            ResourceBindingTarget::Gpio { pin } => Ok(pin),
        }
    }
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            version: RESOURCE_CONFIG_VERSION,
            module_instances: Vec::new(),
            device_instances: Vec::new(),
        }
    }
}

fn deserialize_module_type_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(match value.as_str() {
        "switch" => "core:gpio_switch".to_string(),
        "gpio_output" => "core:gpio_output".to_string(),
        _ => value,
    })
}

fn deserialize_device_type_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(match value.as_str() {
        "switch" => "core:switch".to_string(),
        _ => value,
    })
}

fn deserialize_role_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(match value.as_str() {
        "relay_output" => "core:relay_output".to_string(),
        "wall_trigger_input" => "core:wall_trigger_input".to_string(),
        "output" => "core:output".to_string(),
        _ => value,
    })
}
