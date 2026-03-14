use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use crate::config::types::{DeviceConfig, MqttConfig, WifiConfig};
use crate::error::AppError;

const WIFI_NS: &str = "wifi";
const MQTT_NS: &str = "mqtt";

const KEY_WIFI_SSID: &str = "ssid";
const KEY_WIFI_PASS: &str = "pass";

const KEY_MQTT_HOST: &str = "host";
const KEY_MQTT_PORT: &str = "port";
const KEY_MQTT_USER: &str = "user";
const KEY_MQTT_PASS: &str = "pass";
const KEY_MQTT_CLIENT_ID: &str = "cid";
const KEY_MQTT_BASE_TOPIC: &str = "topic";

pub struct ConfigStore {
    default_nvs: EspDefaultNvsPartition,
}

impl ConfigStore {
    pub fn new() -> Result<Self, AppError> {
        let default_nvs = EspDefaultNvsPartition::take()?;
        Ok(Self { default_nvs })
    }

    pub fn save(&self, cfg: &DeviceConfig) -> Result<(), AppError> {
        {
            let mut wifi_nvs = EspNvs::new(self.default_nvs.clone(), WIFI_NS, true)?;
            wifi_nvs.set_str(KEY_WIFI_SSID, &cfg.wifi.ssid)?;
            wifi_nvs.set_str(KEY_WIFI_PASS, &cfg.wifi.password)?;
        }

        {
            let mut mqtt_nvs = EspNvs::new(self.default_nvs.clone(), MQTT_NS, true)?;
            mqtt_nvs.set_str(KEY_MQTT_HOST, &cfg.mqtt.host)?;
            mqtt_nvs.set_u16(KEY_MQTT_PORT, cfg.mqtt.port)?;
            mqtt_nvs.set_str(KEY_MQTT_USER, &cfg.mqtt.username)?;
            mqtt_nvs.set_str(KEY_MQTT_PASS, &cfg.mqtt.password)?;
            mqtt_nvs.set_str(KEY_MQTT_CLIENT_ID, &cfg.mqtt.client_id)?;
            mqtt_nvs.set_str(KEY_MQTT_BASE_TOPIC, &cfg.mqtt.base_topic)?;
        }

        Ok(())
    }

    pub fn load(&self) -> Result<Option<DeviceConfig>, AppError> {
        let wifi_nvs = EspNvs::new(self.default_nvs.clone(), WIFI_NS, true)?;
        let mqtt_nvs = EspNvs::new(self.default_nvs.clone(), MQTT_NS, true)?;

        let wifi_ssid = get_str(&wifi_nvs, KEY_WIFI_SSID)?;
        let wifi_pass = get_str(&wifi_nvs, KEY_WIFI_PASS)?;
        let mqtt_host = get_str(&mqtt_nvs, KEY_MQTT_HOST)?;
        let mqtt_port = mqtt_nvs.get_u16(KEY_MQTT_PORT)?;
        let mqtt_user = get_str(&mqtt_nvs, KEY_MQTT_USER)?;
        let mqtt_pass = get_str(&mqtt_nvs, KEY_MQTT_PASS)?;
        let mqtt_client_id = get_str(&mqtt_nvs, KEY_MQTT_CLIENT_ID)?;
        let mqtt_base_topic = get_str(&mqtt_nvs, KEY_MQTT_BASE_TOPIC)?;

        let Some(wifi_ssid) = wifi_ssid else { return Ok(None) };
        let Some(wifi_pass) = wifi_pass else { return Ok(None) };
        let Some(mqtt_host) = mqtt_host else { return Ok(None) };
        let Some(mqtt_port) = mqtt_port else { return Ok(None) };
        let Some(mqtt_user) = mqtt_user else { return Ok(None) };
        let Some(mqtt_pass) = mqtt_pass else { return Ok(None) };
        let Some(mqtt_client_id) = mqtt_client_id else { return Ok(None) };
        let Some(mqtt_base_topic) = mqtt_base_topic else { return Ok(None) };

        let cfg = DeviceConfig {
            wifi: WifiConfig {
                ssid: wifi_ssid,
                password: wifi_pass,
            },
            mqtt: MqttConfig {
                host: mqtt_host,
                port: mqtt_port,
                username: mqtt_user,
                password: mqtt_pass,
                client_id: mqtt_client_id,
                base_topic: mqtt_base_topic,
            },
        };

        Ok(Some(cfg))
    }

    pub fn clear_all(&self) -> Result<(), AppError> {
        {
            let mut wifi_nvs = EspNvs::new(self.default_nvs.clone(), WIFI_NS, true)?;
            let _ = wifi_nvs.remove(KEY_WIFI_SSID);
            let _ = wifi_nvs.remove(KEY_WIFI_PASS);
        }

        {
            let mut mqtt_nvs = EspNvs::new(self.default_nvs.clone(), MQTT_NS, true)?;
            let _ = mqtt_nvs.remove(KEY_MQTT_HOST);
            let _ = mqtt_nvs.remove(KEY_MQTT_PORT);
            let _ = mqtt_nvs.remove(KEY_MQTT_USER);
            let _ = mqtt_nvs.remove(KEY_MQTT_PASS);
            let _ = mqtt_nvs.remove(KEY_MQTT_CLIENT_ID);
            let _ = mqtt_nvs.remove(KEY_MQTT_BASE_TOPIC);
        }

        Ok(())
    }
}

fn get_str(nvs: &EspNvs<NvsDefault>, key: &str) -> Result<Option<String>, AppError> {
    let mut buf = [0_u8; 256];

    match nvs.get_str(key, &mut buf) {
        Ok(Some(value)) => Ok(Some(value.to_string())),
        Ok(None) => Ok(None),
        Err(e) => Err(AppError::from(e)),
    }
}
