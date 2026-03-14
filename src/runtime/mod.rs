use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::mqtt::client::QoS;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::{error, info};

use crate::app::{detect_boot_mode, BootMode};
use crate::error::AppError;
use crate::http::{self, ProvisioningController};
use crate::mqtt;
use crate::storage::nvs::ConfigStore;
use crate::wifi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Booting,
    Provisioning,
    NormalStartup,
    WifiConnecting,
    MqttConnecting,
    Operational,
    RestartPending,
    Degraded,
}

#[derive(Clone, Default)]
pub struct RuntimeSignals {
    restart_pending: Arc<AtomicBool>,
}

impl RuntimeSignals {
    pub fn mark_restart_pending(&self) {
        self.restart_pending.store(true, Ordering::SeqCst);
    }

    pub fn is_restart_pending(&self) -> bool {
        self.restart_pending.load(Ordering::SeqCst)
    }
}

pub struct AppController {
    state: AppState,
    signals: RuntimeSignals,
    nvs: EspDefaultNvsPartition,
    store: ConfigStore,
}

impl AppController {
    pub fn new(nvs: EspDefaultNvsPartition) -> Self {
        let store = ConfigStore::with_partition(nvs.clone());

        Self {
            state: AppState::Booting,
            signals: RuntimeSignals::default(),
            nvs,
            store,
        }
    }

    pub fn run(mut self) -> Result<(), AppError> {
        self.transition_to(AppState::Booting);
        self.clear_legacy_demo_config()?;

        match detect_boot_mode(&self.store)? {
            BootMode::Normal => self.run_normal_mode(),
            BootMode::Provisioning => self.run_provisioning_mode(),
        }
    }

    fn clear_legacy_demo_config(&self) -> Result<(), AppError> {
        if let Some(cfg) = self.store.load()? {
            if cfg.is_legacy_demo_seed() {
                info!("Removing legacy demo config from NVS");
                self.store.clear_all()?;
            }
        }

        Ok(())
    }

    fn run_normal_mode(&mut self) -> Result<(), AppError> {
        self.transition_to(AppState::NormalStartup);

        let result = (|| -> Result<(), AppError> {
            info!("Complete config found in NVS, entering normal mode");

            let cfg = self
                .store
                .load()?
                .ok_or_else(|| AppError::Message("Config disappeared unexpectedly".into()))?;
            let peripherals = take_peripherals()?;
            let sys_loop = EspSystemEventLoop::take()?;

            info!("Wi-Fi SSID: {}", cfg.wifi.ssid);
            info!("MQTT host: {}:{}", cfg.mqtt.host, cfg.mqtt.port);

            self.transition_to(AppState::WifiConnecting);
            let _wifi =
                wifi::connect_sta(peripherals.modem, sys_loop, self.nvs.clone(), &cfg.wifi)?;

            self.transition_to(AppState::MqttConnecting);
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

            let _http_server = http::start_server(self.store_partition(), self.signals.clone())?;
            self.transition_to(AppState::Operational);

            self.idle_forever()
        })();

        self.finish_result(result)
    }

    fn run_provisioning_mode(&mut self) -> Result<(), AppError> {
        let result = (|| -> Result<(), AppError> {
            info!("No complete config found, entering provisioning mode");

            let peripherals = take_peripherals()?;
            let sys_loop = EspSystemEventLoop::take()?;

            self.transition_to(AppState::Provisioning);

            let mut wifi = wifi::start_ap(peripherals.modem, sys_loop, self.nvs.clone())?;
            let ssid = wifi.ap_ssid().to_string();
            let cached_networks = wifi.scan_networks()?;
            let controller =
                ProvisioningController::new(self.store_partition(), ssid.clone(), cached_networks);
            let _http_server =
                http::start_captive_portal_server(controller.clone(), self.signals.clone())?;

            info!("Provisioning AP ready on SSID: {ssid}");

            loop {
                if self.signals.is_restart_pending() && self.state != AppState::RestartPending {
                    self.transition_to(AppState::RestartPending);
                }

                if let Some(cfg) = controller.take_pending_wifi_test()? {
                    self.transition_to(AppState::WifiConnecting);
                    controller.mark_wifi_test_running(&cfg.ssid)?;

                    match wifi.test_sta_connection(&cfg) {
                        Ok(ip) => controller.mark_wifi_test_success(cfg, ip)?,
                        Err(error) => controller.mark_wifi_test_error(&error)?,
                    }

                    if !self.signals.is_restart_pending() {
                        self.transition_to(AppState::Provisioning);
                    }
                }

                if let Some(cfg) = controller.take_pending_mqtt_test()? {
                    self.transition_to(AppState::MqttConnecting);
                    controller.mark_mqtt_test_running(&cfg.host)?;

                    match controller.wifi_for_mqtt_test() {
                        Ok(_) => match mqtt::test_connection(&cfg, Duration::from_secs(15)) {
                            Ok(()) => controller.mark_mqtt_test_success(cfg)?,
                            Err(error) => controller.mark_mqtt_test_error(&error)?,
                        },
                        Err(error) => controller.mark_mqtt_test_error(&error)?,
                    }

                    if !self.signals.is_restart_pending() {
                        self.transition_to(AppState::Provisioning);
                    }
                }

                thread::sleep(Duration::from_millis(200));
            }
        })();

        self.finish_result(result)
    }

    fn finish_result(&mut self, result: Result<(), AppError>) -> Result<(), AppError> {
        if let Err(error) = &result {
            self.transition_to(AppState::Degraded);
            error!("Application runtime degraded: {error}");
        }

        result
    }

    fn store_partition(&self) -> ConfigStore {
        ConfigStore::with_partition(self.nvs.clone())
    }

    fn transition_to(&mut self, next: AppState) {
        if self.state != next {
            info!("App state transition: {:?} -> {:?}", self.state, next);
            self.state = next;
        }
    }

    fn idle_forever(&self) -> Result<(), AppError> {
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }
}

fn take_peripherals() -> Result<Peripherals, AppError> {
    Peripherals::take()
        .map_err(|e| AppError::Message(format!("Failed to take ESP peripherals: {e:?}")))
}
