mod indicator;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::mqtt::client::QoS;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::{error, info, warn};

use crate::app::{detect_boot_mode, BootMode};
use crate::board::BoardProfile;
use crate::error::AppError;
use crate::gpio::GpioManager;
use crate::http::{self, ProvisioningController};
use crate::modules::{ModuleCommand, ModuleManager};
use crate::mqtt;
use crate::mqtt::contract::{ConfigOperationResultPayload, ContractAction, MqttContract};
use crate::runtime::indicator::{ErrorKind, IndicatorMode, LedIndicator};
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
    error_kind: ErrorKind,
    recovering_from_error: bool,
    signals: RuntimeSignals,
    nvs: EspDefaultNvsPartition,
    store: ConfigStore,
    gpio_manager: GpioManager,
    led_indicator: LedIndicator,
    wifi_retry_at: Option<Instant>,
    mqtt_retry_at: Option<Instant>,
}

enum RuntimeCommand {
    Contract(ContractAction),
    Module(ModuleCommand),
}

impl AppController {
    pub fn new(nvs: EspDefaultNvsPartition) -> Self {
        let store = ConfigStore::with_partition(nvs.clone());

        Self {
            state: AppState::Booting,
            error_kind: ErrorKind::Unknown,
            recovering_from_error: false,
            signals: RuntimeSignals::default(),
            nvs,
            store,
            gpio_manager: GpioManager::new(BoardProfile::esp32_devkit_v1()),
            led_indicator: LedIndicator::new_onboard().expect("failed to initialize onboard LED"),
            wifi_retry_at: None,
            mqtt_retry_at: None,
        }
    }

    pub fn run(mut self) -> Result<(), AppError> {
        self.transition_to(AppState::Booting);
        self.led_indicator.set_mode(IndicatorMode::Off);
        self.clear_legacy_demo_config()?;
        self.initialize_resource_runtime()?;

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

    fn initialize_resource_runtime(&mut self) -> Result<(), AppError> {
        let resources = self.store.load_resources()?;
        match self.gpio_manager.validate_config(&resources) {
            Ok(()) => {
                let snapshot = self.gpio_manager.snapshot(&resources);
                info!(
                    "GPIO/resource runtime ready for board '{}' with {} module binding(s)",
                    snapshot.board.name,
                    snapshot.module_instances.len()
                );
            }
            Err(error) => {
                error!("Ignoring invalid stored GPIO/resource configuration: {error}");
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
            let mut resources = self.store.load_resources()?;
            let contract = MqttContract::new(&cfg.mqtt, BoardProfile::esp32_devkit_v1());
            let (command_tx, command_rx) = mpsc::channel();

            self.transition_to(AppState::WifiConnecting);
            let mut wifi =
                wifi::create_sta(peripherals.modem, sys_loop, self.nvs.clone(), &cfg.wifi)?;
            self.connect_wifi_with_retry(&mut wifi)?;

            let mut module_manager = match self.load_runtime_modules(&resources) {
                Ok(manager) => manager,
                Err(error) => {
                    error!("Failed to initialize runtime modules: {error}");
                    ModuleManager::empty()
                }
            };

            self.transition_to(AppState::MqttConnecting);
            let mut mqtt = Some(self.establish_mqtt_session(
                &cfg.mqtt,
                &contract,
                &resources,
                &mut module_manager,
                &command_tx,
                &wifi,
            )?);

            let _http_server = http::start_server(self.store_partition(), self.signals.clone())?;
            self.transition_to(AppState::Operational);

            self.run_operational_loop(
                &mut wifi,
                &mut mqtt,
                &cfg.mqtt,
                &contract,
                &command_tx,
                &command_rx,
                &mut resources,
                &mut module_manager,
            )
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
            let error_kind = self.error_kind_for_current_state();
            self.error_kind = error_kind;
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
            if next == AppState::Operational {
                self.recovering_from_error = false;
            }
            self.update_indicator_for_state();
        }
    }

    fn idle_forever(&self) -> Result<(), AppError> {
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }

    fn run_operational_loop(
        &mut self,
        wifi: &mut esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
        mqtt: &mut Option<mqtt::MqttManager>,
        mqtt_config: &crate::config::types::MqttConfig,
        contract: &MqttContract,
        command_tx: &mpsc::Sender<RuntimeCommand>,
        command_rx: &mpsc::Receiver<RuntimeCommand>,
        resources: &mut crate::config::types::ResourceConfig,
        module_manager: &mut ModuleManager,
    ) -> Result<(), AppError> {
        loop {
            self.check_runtime_connectivity(
                wifi,
                mqtt,
                mqtt_config,
                contract,
                command_tx,
                resources,
                module_manager,
            )?;

            module_manager.poll(mqtt.as_mut(), contract.topics())?;

            while let Ok(action) = command_rx.try_recv() {
                if let Some(mqtt) = mqtt.as_mut() {
                    self.handle_runtime_command(mqtt, contract, resources, module_manager, action)?;
                }
            }

            thread::sleep(Duration::from_millis(200));
        }
    }

    fn check_runtime_connectivity(
        &mut self,
        wifi: &mut esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
        mqtt: &mut Option<mqtt::MqttManager>,
        mqtt_config: &crate::config::types::MqttConfig,
        contract: &MqttContract,
        command_tx: &mpsc::Sender<RuntimeCommand>,
        resources: &crate::config::types::ResourceConfig,
        module_manager: &mut ModuleManager,
    ) -> Result<(), AppError> {
        if !wifi.is_connected()? || !wifi.is_up()? {
            self.error_kind = ErrorKind::Wifi;
            self.recovering_from_error = true;
            self.transition_to(AppState::Degraded);
            let now = Instant::now();
            let retry_at = *self
                .wifi_retry_at
                .get_or_insert_with(|| now + Duration::from_secs(2));

            if now < retry_at {
                return Ok(());
            }

            // Stop MQTT retries while the STA link is down.
            let _ = mqtt.take();
            self.transition_to(AppState::WifiConnecting);

            if let Err(error) = wifi.connect().and_then(|_| wifi.wait_netif_up()) {
                warn!("Wi-Fi reconnect attempt failed: {error}");
                self.error_kind = ErrorKind::Wifi;
                self.transition_to(AppState::Degraded);
                self.wifi_retry_at = Some(Instant::now() + Duration::from_secs(2));
                return Ok(());
            }

            self.wifi_retry_at = None;
            self.mqtt_retry_at = None;

            self.transition_to(AppState::MqttConnecting);
            match self.establish_mqtt_session(
                mqtt_config,
                contract,
                resources,
                module_manager,
                command_tx,
                wifi,
            ) {
                Ok(session) => {
                    *mqtt = Some(session);
                    self.transition_to(AppState::Operational);
                }
                Err(error) => {
                    warn!("MQTT reconnect after Wi-Fi recovery failed: {error}");
                    self.error_kind = classify_wifi_first(wifi)?;
                    self.recovering_from_error = true;
                    self.transition_to(AppState::Degraded);
                    self.mqtt_retry_at = Some(Instant::now() + Duration::from_secs(2));
                }
            }

            return Ok(());
        }

        if mqtt
            .as_ref()
            .is_some_and(|mqtt| mqtt.is_connected().ok() == Some(false))
        {
            self.error_kind = ErrorKind::Mqtt;
            self.recovering_from_error = true;
            self.transition_to(AppState::Degraded);
            let now = Instant::now();
            let retry_at = *self
                .mqtt_retry_at
                .get_or_insert_with(|| now + Duration::from_secs(2));

            if now < retry_at {
                return Ok(());
            }

            self.transition_to(AppState::MqttConnecting);

            let Some(active_mqtt) = mqtt.as_mut() else {
                return Ok(());
            };

            if let Err(error) = active_mqtt.wait_until_connected(Duration::from_secs(15)) {
                self.error_kind = classify_wifi_first(wifi)?;
                warn!("MQTT reconnect attempt failed: {error}");
                self.transition_to(AppState::Degraded);
                self.mqtt_retry_at = Some(Instant::now() + Duration::from_secs(2));
                return Ok(());
            }
            self.mqtt_retry_at = None;
            contract.publish_birth(active_mqtt, &self.gpio_manager.snapshot(resources))?;
            module_manager.publish_initial_states(active_mqtt, contract.topics())?;
            self.transition_to(AppState::Operational);
        }

        Ok(())
    }

    fn handle_runtime_command(
        &mut self,
        mqtt: &mut mqtt::MqttManager,
        contract: &MqttContract,
        resources: &mut crate::config::types::ResourceConfig,
        module_manager: &mut ModuleManager,
        action: RuntimeCommand,
    ) -> Result<(), AppError> {
        match action {
            RuntimeCommand::Contract(action) => {
                self.handle_contract_action(mqtt, contract, resources, module_manager, action)
            }
            RuntimeCommand::Module(command) => {
                if let Err(error) = module_manager.handle_command(mqtt, contract.topics(), command)
                {
                    warn!("Module command failed: {error}");
                }
                Ok(())
            }
        }
    }

    fn handle_contract_action(
        &mut self,
        mqtt: &mut mqtt::MqttManager,
        contract: &MqttContract,
        resources: &mut crate::config::types::ResourceConfig,
        module_manager: &mut ModuleManager,
        action: ContractAction,
    ) -> Result<(), AppError> {
        match action {
            ContractAction::GetConfig { request_id } => {
                let snapshot = self.gpio_manager.snapshot(resources);
                contract.publish_birth(mqtt, &snapshot)?;
                contract.publish_config_result(
                    mqtt,
                    &MqttContract::ok_result(
                        "get_config",
                        request_id,
                        "Published current board and resource configuration.",
                    ),
                )?;
            }
            ContractAction::ValidateResources {
                request_id,
                resources: proposed_resources,
            } => {
                let result = match self.gpio_manager.validate_config(&proposed_resources) {
                    Ok(()) => MqttContract::ok_result(
                        "validate_resources",
                        request_id,
                        "Resource configuration is valid.",
                    ),
                    Err(error) => MqttContract::error_result(
                        "validate_resources",
                        request_id,
                        error.to_string(),
                    ),
                };

                contract.publish_config_result(mqtt, &result)?;
            }
            ContractAction::SetResources {
                request_id,
                resources: proposed_resources,
            } => match self.gpio_manager.validate_config(&proposed_resources) {
                Ok(()) => {
                    let reloaded_modules = self.load_runtime_modules(&proposed_resources)?;
                    self.store.save_resources(&proposed_resources)?;
                    *resources = proposed_resources;
                    *module_manager = reloaded_modules;

                    let snapshot = self.gpio_manager.snapshot(resources);
                    contract.publish_resources_snapshot(mqtt, &snapshot)?;
                    module_manager.publish_initial_states(mqtt, contract.topics())?;
                    contract.publish_config_result(
                        mqtt,
                        &MqttContract::ok_result(
                            "set_resources",
                            request_id,
                            "Resource configuration saved successfully.",
                        ),
                    )?;
                }
                Err(error) => {
                    contract.publish_config_result(
                        mqtt,
                        &MqttContract::error_result("set_resources", request_id, error.to_string()),
                    )?;
                }
            },
            ContractAction::PublishResult(payload) => {
                self.publish_contract_result(mqtt, contract, payload)?;
            }
        }

        Ok(())
    }

    fn publish_contract_result(
        &mut self,
        mqtt: &mut mqtt::MqttManager,
        contract: &MqttContract,
        payload: ConfigOperationResultPayload,
    ) -> Result<(), AppError> {
        contract.publish_config_result(mqtt, &payload)
    }

    fn load_runtime_modules(
        &mut self,
        resources: &crate::config::types::ResourceConfig,
    ) -> Result<ModuleManager, AppError> {
        let mut gpio_manager = GpioManager::new(BoardProfile::esp32_devkit_v1());
        let module_manager = ModuleManager::load(resources, &mut gpio_manager)?;
        self.gpio_manager = gpio_manager;
        Ok(module_manager)
    }

    fn connect_wifi_with_retry(
        &mut self,
        wifi: &mut esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
    ) -> Result<(), AppError> {
        loop {
            match wifi::connect_sta_existing(wifi) {
                Ok(()) => {
                    self.wifi_retry_at = None;
                    return Ok(());
                }
                Err(error) => {
                    warn!("Initial Wi-Fi connect attempt failed: {error}");
                    self.error_kind = ErrorKind::Wifi;
                    self.transition_to(AppState::Degraded);
                    thread::sleep(Duration::from_secs(2));
                    self.transition_to(AppState::WifiConnecting);
                }
            }
        }
    }

    fn update_indicator_for_state(&self) {
        let mode = if self.recovering_from_error
            && matches!(
                self.state,
                AppState::Degraded | AppState::WifiConnecting | AppState::MqttConnecting
            ) {
            IndicatorMode::Error(self.error_kind)
        } else {
            match self.state {
                AppState::Provisioning => IndicatorMode::Provisioning,
                AppState::NormalStartup | AppState::WifiConnecting | AppState::MqttConnecting => {
                    IndicatorMode::NormalStartup
                }
                AppState::Operational => IndicatorMode::Operational,
                AppState::Degraded => IndicatorMode::Error(self.error_kind),
                AppState::Booting | AppState::RestartPending => IndicatorMode::Off,
            }
        };

        self.led_indicator.set_mode(mode);
    }

    fn error_kind_for_current_state(&self) -> ErrorKind {
        match self.state {
            AppState::WifiConnecting => ErrorKind::Wifi,
            AppState::MqttConnecting => ErrorKind::Mqtt,
            _ => ErrorKind::Unknown,
        }
    }

    fn establish_mqtt_session(
        &mut self,
        mqtt_config: &crate::config::types::MqttConfig,
        contract: &MqttContract,
        resources: &crate::config::types::ResourceConfig,
        module_manager: &mut ModuleManager,
        command_tx: &mpsc::Sender<RuntimeCommand>,
        wifi: &esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
    ) -> Result<mqtt::MqttManager, AppError> {
        let last_will = contract.last_will()?;
        let command_topic = contract.topics().command_wildcard();
        let module_command_topic = contract.topics().module_command_wildcard();
        let mut mqtt = mqtt::MqttManager::connect_with_last_will(mqtt_config, Some(&last_will))?;

        if let Err(error) = mqtt.wait_until_connected(Duration::from_secs(15)) {
            self.error_kind = classify_wifi_first(wifi)?;
            return Err(error);
        }

        mqtt.subscribe(command_topic.as_str(), QoS::AtMostOnce, {
            let command_tx = command_tx.clone();
            let contract = contract.clone();

            move |message| {
                let _ = command_tx.send(RuntimeCommand::Contract(contract.parse_action(message)));
            }
        })?;
        mqtt.subscribe(module_command_topic.as_str(), QoS::AtMostOnce, {
            let command_tx = command_tx.clone();
            let topics = contract.topics().clone();

            move |message| {
                if let Some(module_id) = topics.parse_module_command_topic(&message.topic) {
                    let _ = command_tx.send(RuntimeCommand::Module(ModuleCommand {
                        module_id,
                        payload: message.payload.clone(),
                    }));
                }
            }
        })?;
        contract.publish_birth(&mut mqtt, &self.gpio_manager.snapshot(resources))?;
        module_manager.publish_initial_states(&mut mqtt, contract.topics())?;

        Ok(mqtt)
    }
}

fn take_peripherals() -> Result<Peripherals, AppError> {
    Peripherals::take()
        .map_err(|e| AppError::Message(format!("Failed to take ESP peripherals: {e:?}")))
}

fn classify_wifi_first(
    wifi: &esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
) -> Result<ErrorKind, AppError> {
    if !wifi.is_connected()? || !wifi.is_up()? {
        Ok(ErrorKind::Wifi)
    } else {
        Ok(ErrorKind::Mqtt)
    }
}
