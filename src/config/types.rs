use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceConfig {
    #[serde(default)]
    pub module_instances: Vec<ModuleInstanceConfig>,
    #[serde(default)]
    pub device_instances: Vec<DeviceInstanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInstanceConfig {
    pub id: String,
    pub module_type: ModuleType,
    pub display_name: Option<String>,
    pub bindings: Vec<PinBindingConfig>,
    #[serde(default)]
    pub settings: ModuleSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInstanceConfig {
    pub id: String,
    pub device_type: DeviceType,
    pub display_name: Option<String>,
    pub driver_module_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleType {
    Switch,
    GpioOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Switch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleRole {
    RelayOutput,
    WallTriggerInput,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceUsage {
    Input,
    Output,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinBindingConfig {
    pub role: ModuleRole,
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

    pub fn is_legacy_demo_seed(&self) -> bool {
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

impl ModuleType {
    pub fn required_roles(self) -> &'static [ModuleRole] {
        match self {
            Self::Switch => &[ModuleRole::RelayOutput],
            Self::GpioOutput => &[ModuleRole::Output],
        }
    }
}

impl ModuleRole {
    pub fn usage(self) -> ResourceUsage {
        match self {
            Self::RelayOutput => ResourceUsage::Output,
            Self::WallTriggerInput => ResourceUsage::Input,
            Self::Output => ResourceUsage::Output,
        }
    }
}

impl PinBindingConfig {
    pub fn pin(&self) -> Result<u8, crate::error::AppError> {
        match self.target {
            ResourceBindingTarget::Gpio { pin } => Ok(pin),
        }
    }
}
