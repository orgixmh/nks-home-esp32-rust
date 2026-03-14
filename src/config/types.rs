#[derive(Debug, Clone)]
pub struct WifiConfig {
    pub ssid: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub client_id: String,
    pub base_topic: String,
}

#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub wifi: WifiConfig,
    pub mqtt: MqttConfig,
}

impl DeviceConfig {
    pub fn is_complete(&self) -> bool {
        !self.wifi.ssid.trim().is_empty()
            && !self.mqtt.host.trim().is_empty()
            && self.mqtt.port > 0
            && !self.mqtt.client_id.trim().is_empty()
            && !self.mqtt.base_topic.trim().is_empty()
    }
}
