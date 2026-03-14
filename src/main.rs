mod app;
mod config;
mod error;
mod http;
mod mqtt;
mod storage;
mod wifi;

//use esp_idf_svc::sys as _;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::mqtt::client::QoS;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;
use std::thread;
use std::time::Duration;

use crate::app::{detect_boot_mode, BootMode};
use crate::error::AppError;
use crate::storage::nvs::ConfigStore;

fn main() -> Result<(), AppError> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Booting nks-smart-home...");
    let nvs = EspDefaultNvsPartition::take()?;
    let store = ConfigStore::with_partition(nvs.clone());

    if let Some(cfg) = store.load()? {
        if cfg.is_legacy_demo_seed() {
            info!("Removing legacy demo config from NVS");
            store.clear_all()?;
        }
    }

    match detect_boot_mode(&store)? {
        BootMode::Normal => {
            info!("Complete config found in NVS, entering normal mode");

            let cfg = store.load()?.expect("config disappeared unexpectedly");
            let peripherals = Peripherals::take()
                .map_err(|e| AppError::Message(format!("Failed to take ESP peripherals: {e:?}")))?;
            let sys_loop = EspSystemEventLoop::take()?;

            info!("Wi-Fi SSID: {}", cfg.wifi.ssid);
            info!("MQTT host: {}:{}", cfg.mqtt.host, cfg.mqtt.port);

            let _wifi = wifi::connect_sta(peripherals.modem, sys_loop, nvs, &cfg.wifi)?;
            let mut _mqtt = mqtt::MqttManager::connect(&cfg.mqtt)?;
            let mqtt_status_topic = format!("{}/status", cfg.mqtt.base_topic);
            let mqtt_command_topic = format!("{}/cmd/#", cfg.mqtt.base_topic);

            _mqtt.wait_until_connected(Duration::from_secs(15))?;
            _mqtt.subscribe(&mqtt_command_topic, QoS::AtMostOnce, |message| {
                info!(
                    "MQTT message on {}: {}",
                    message.topic,
                    String::from_utf8_lossy(&message.payload)
                );
            })?;
            _mqtt.publish(
                &mqtt_status_topic,
                br#"{"status":"online"}"#,
                QoS::AtLeastOnce,
                false,
            )?;
            let _http_server = http::start_server(store)?;

            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        BootMode::Provisioning => {
            info!("No complete config found, entering provisioning mode");
            let peripherals = Peripherals::take()
                .map_err(|e| AppError::Message(format!("Failed to take ESP peripherals: {e:?}")))?;
            let sys_loop = EspSystemEventLoop::take()?;

            let mut wifi = wifi::start_ap(peripherals.modem, sys_loop, nvs)?;
            let ssid = wifi.ap_ssid().to_string();
            let cached_networks = wifi.scan_networks()?;
            let controller =
                http::ProvisioningController::new(store, ssid.clone(), cached_networks);
            let _http_server = http::start_captive_portal_server(controller.clone())?;

            info!("Provisioning AP ready on SSID: {ssid}");

            loop {
                if let Some(cfg) = controller.take_pending_wifi_test()? {
                    controller.mark_wifi_test_running(&cfg.ssid)?;

                    match wifi.test_sta_connection(&cfg) {
                        Ok(ip) => controller.mark_wifi_test_success(cfg, ip)?,
                        Err(error) => controller.mark_wifi_test_error(&error)?,
                    }
                }

                if let Some(cfg) = controller.take_pending_mqtt_test()? {
                    controller.mark_mqtt_test_running(&cfg.host)?;

                    match controller.wifi_for_mqtt_test() {
                        Ok(_) => match mqtt::test_connection(&cfg, Duration::from_secs(15)) {
                            Ok(()) => controller.mark_mqtt_test_success(cfg)?,
                            Err(error) => controller.mark_mqtt_test_error(&error)?,
                        },
                        Err(error) => controller.mark_mqtt_test_error(&error)?,
                    }
                }

                thread::sleep(Duration::from_millis(200));
            }
        }
    }

    Ok(())
}
