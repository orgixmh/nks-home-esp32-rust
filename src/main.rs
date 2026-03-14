mod app;
mod config;
mod error;
mod storage;

//use esp_idf_svc::sys as _;
use log::info;

use crate::app::{detect_boot_mode, BootMode};
use crate::error::AppError;
use crate::storage::nvs::ConfigStore;
use crate::config::types::{DeviceConfig, WifiConfig, MqttConfig};

fn main() -> Result<(), AppError> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Booting nks-smart-home...");

    match detect_boot_mode()? {
        BootMode::Normal => {
            info!("Complete config found in NVS, entering normal mode");

            let store = ConfigStore::new()?;
            let cfg = store.load()?.expect("config disappeared unexpectedly");

            info!("Wi-Fi SSID: {}", cfg.wifi.ssid);
            info!("MQTT host: {}:{}", cfg.mqtt.host, cfg.mqtt.port);

            // next step:
            // wifi::connect(&cfg.wifi)?;
            // mqtt::connect(&cfg.mqtt)?;
        }
        BootMode::Provisioning => {
            info!("No complete config found, entering provisioning mode");

            let demo_cfg = DeviceConfig {
                wifi: WifiConfig {
                    ssid: "eps-rust-test".into(),
                    password: "asdfg43v34t34f34t3".into(),
                },
                mqtt: MqttConfig {
                    host: "10.0.0.1".into(),
                    port: 1883,
                    username: "testuser".into(),
                    password: "testpassword".into(),
                    client_id: "esp32-test-node".into(),
                    base_topic: "nks/home/test-node".into(),
                },
            };

            let store = ConfigStore::new()?;
            store.save(&demo_cfg)?;
            info!("Demo config saved to NVS. Reboot to test normal mode path.");
        }
    }

    Ok(())
}
