mod app;
mod config;
mod error;
mod storage;
mod wifi;

//use esp_idf_svc::sys as _;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;
use std::thread;
use std::time::Duration;

use crate::app::{detect_boot_mode, BootMode};
use crate::config::types::{DeviceConfig, MqttConfig, WifiConfig};
use crate::error::AppError;
use crate::storage::nvs::ConfigStore;

fn main() -> Result<(), AppError> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Booting nks-smart-home...");
    let nvs = EspDefaultNvsPartition::take()?;
    let store = ConfigStore::with_partition(nvs.clone());

    match detect_boot_mode(&store)? {
        BootMode::Normal => {
            info!("Complete config found in NVS, entering normal mode");

            let cfg = store.load()?.expect("config disappeared unexpectedly");
            drop(store);
            let peripherals = Peripherals::take()
                .map_err(|e| AppError::Message(format!("Failed to take ESP peripherals: {e:?}")))?;
            let sys_loop = EspSystemEventLoop::take()?;

            info!("Wi-Fi SSID: {}", cfg.wifi.ssid);
            info!("MQTT host: {}:{}", cfg.mqtt.host, cfg.mqtt.port);

            let _wifi = wifi::connect_sta(peripherals.modem, sys_loop, nvs, &cfg.wifi)?;
            // mqtt::connect(&cfg.mqtt)?;

            loop {
                thread::sleep(Duration::from_secs(60));
            }
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

            store.save(&demo_cfg)?;
            info!("Demo config saved to NVS. Reboot to test normal mode path.");
        }
    }

    Ok(())
}
