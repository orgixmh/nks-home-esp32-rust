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
use crate::config::types::ResourceConfig;
use crate::devices::{DeviceCommand, DeviceRegistry};
use crate::error::AppError;
use crate::gpio::GpioManager;
use crate::http::{self, ProvisioningController};
use crate::modules::{ModuleCommand, ModuleManager};
use crate::mqtt;
use crate::mqtt::contract::{ConfigOperationResultPayload, ContractAction, MqttContract};
use crate::runtime::indicator::{IndicatorMode, LedIndicator};
use crate::schemas::SchemaRegistry;
use crate::storage::nvs::ConfigStore;
use crate::wifi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationalStatus {
    Provisioning,
    Operational,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedReason {
    WifiDisconnected,
    MqttDisconnected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAction {
    WifiConnecting,
    MqttConnecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeState {
    pub status: OperationalStatus,
    pub reason: Option<DegradedReason>,
    pub action: Option<RuntimeAction>,
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
    state: RuntimeState,
    signals: RuntimeSignals,
    nvs: EspDefaultNvsPartition,
    store: ConfigStore,
    gpio_manager: GpioManager,
    schema_registry: SchemaRegistry,
    led_indicator: LedIndicator,
    wifi_retry_at: Option<Instant>,
    mqtt_retry_at: Option<Instant>,
}

enum RuntimeCommand {
    Contract(ContractAction),
    Device(DeviceCommand),
    Module(ModuleCommand),
}

impl AppController {
    pub fn new(nvs: EspDefaultNvsPartition) -> Self {
        let store = ConfigStore::with_partition(nvs.clone());

        Self {
            state: RuntimeState {
                status: OperationalStatus::Degraded,
                reason: Some(DegradedReason::Unknown),
                action: None,
            },
            signals: RuntimeSignals::default(),
            nvs,
            store,
            gpio_manager: GpioManager::new(BoardProfile::esp32_devkit_v1()),
            schema_registry: SchemaRegistry::built_in().expect("failed to initialize schemas"),
            led_indicator: LedIndicator::new_onboard().expect("failed to initialize onboard LED"),
            wifi_retry_at: None,
            mqtt_retry_at: None,
        }
    }

    pub fn run(mut self) -> Result<(), AppError> {
        self.led_indicator.set_mode(IndicatorMode::Off);
        self.led_indicator.tick();
        self.clear_demo_seed_config()?;
        self.initialize_resource_runtime()?;

        match detect_boot_mode(&self.store)? {
            BootMode::Normal => self.run_normal_mode(),
            BootMode::Provisioning => self.run_provisioning_mode(),
        }
    }

    fn clear_demo_seed_config(&self) -> Result<(), AppError> {
        if let Some(cfg) = self.store.load()? {
            if cfg.is_demo_seed_config() {
                info!("Removing demo seed config from NVS");
                self.store.clear_all()?;
            }
        }

        Ok(())
    }

    fn initialize_resource_runtime(&mut self) -> Result<(), AppError> {
        let resources = self.load_effective_resources()?;
        match self
            .gpio_manager
            .validate_config(&resources, &self.schema_registry)
            .and_then(|_| DeviceRegistry::validate_config(&resources, &self.schema_registry))
        {
            Ok(()) => {
                let snapshot = self
                    .gpio_manager
                    .snapshot(&resources, &self.schema_registry);
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
        self.set_operational_action(Some(RuntimeAction::WifiConnecting));

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
            let mut resources = self.load_effective_resources()?;
            let contract = MqttContract::new(
                &cfg.mqtt,
                BoardProfile::esp32_devkit_v1(),
                &self.schema_registry,
            );
            let (command_tx, command_rx) = mpsc::channel();

            self.set_operational_action(Some(RuntimeAction::WifiConnecting));
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
            let mut device_registry = match DeviceRegistry::load(&resources, &self.schema_registry)
            {
                Ok(registry) => registry,
                Err(error) => {
                    error!("Failed to initialize logical devices: {error}");
                    DeviceRegistry::empty()
                }
            };

            self.set_operational_action(Some(RuntimeAction::MqttConnecting));
            let mut mqtt = Some(self.establish_mqtt_session(
                &cfg.mqtt,
                &contract,
                &resources,
                &mut module_manager,
                &device_registry,
                &command_tx,
                &wifi,
            )?);

            let _http_server = http::start_server(self.store_partition(), self.signals.clone())?;
            self.set_operational_action(None);

            self.run_operational_loop(
                &mut wifi,
                &mut mqtt,
                &cfg.mqtt,
                &contract,
                &command_tx,
                &command_rx,
                &mut resources,
                &mut module_manager,
                &mut device_registry,
            )
        })();

        self.finish_result(result)
    }

    fn run_provisioning_mode(&mut self) -> Result<(), AppError> {
        let result = (|| -> Result<(), AppError> {
            info!("No complete config found, entering provisioning mode");

            let peripherals = take_peripherals()?;
            let sys_loop = EspSystemEventLoop::take()?;

            self.set_provisioning_action(None);

            let mut wifi = wifi::start_ap(peripherals.modem, sys_loop, self.nvs.clone())?;
            let ssid = wifi.ap_ssid().to_string();
            let cached_networks = wifi.scan_networks()?;
            let controller =
                ProvisioningController::new(self.store_partition(), ssid.clone(), cached_networks);
            let _http_server =
                http::start_captive_portal_server(controller.clone(), self.signals.clone())?;

            info!("Provisioning AP ready on SSID: {ssid}");

            loop {
                if self.signals.is_restart_pending() {
                    self.led_indicator.set_mode(IndicatorMode::Off);
                    self.led_indicator.tick();
                }

                if let Some(cfg) = controller.take_pending_wifi_test()? {
                    self.set_provisioning_action(Some(RuntimeAction::WifiConnecting));
                    controller.mark_wifi_test_running(&cfg.ssid)?;

                    match wifi.test_sta_connection(&cfg) {
                        Ok(ip) => controller.mark_wifi_test_success(cfg, ip)?,
                        Err(error) => controller.mark_wifi_test_error(&error)?,
                    }

                    if !self.signals.is_restart_pending() {
                        self.set_provisioning_action(None);
                    }
                }

                if let Some(cfg) = controller.take_pending_mqtt_test()? {
                    self.set_provisioning_action(Some(RuntimeAction::MqttConnecting));
                    controller.mark_mqtt_test_running(&cfg.host)?;

                    match controller.wifi_for_mqtt_test() {
                        Ok(_) => match mqtt::test_connection(&cfg, Duration::from_secs(15)) {
                            Ok(()) => controller.mark_mqtt_test_success(cfg)?,
                            Err(error) => controller.mark_mqtt_test_error(&error)?,
                        },
                        Err(error) => controller.mark_mqtt_test_error(&error)?,
                    }

                    if !self.signals.is_restart_pending() {
                        self.set_provisioning_action(None);
                    }
                }

                self.sleep_with_indicator(Duration::from_millis(200));
            }
        })();

        self.finish_result(result)
    }

    fn finish_result(&mut self, result: Result<(), AppError>) -> Result<(), AppError> {
        if let Err(error) = &result {
            self.set_degraded(self.reason_for_current_action(), None);
            error!("Application runtime degraded: {error}");
        }

        result
    }

    fn store_partition(&self) -> ConfigStore {
        ConfigStore::with_partition(self.nvs.clone())
    }

    fn idle_forever(&self) -> Result<(), AppError> {
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }

    fn set_runtime_state(&mut self, next: RuntimeState) {
        if self.state != next {
            info!("Runtime state transition: {:?} -> {:?}", self.state, next);
            self.state = next;
            self.led_indicator.set_mode(IndicatorMode::State(next));
            self.led_indicator.tick();
        }
    }

    fn set_operational_action(&mut self, action: Option<RuntimeAction>) {
        self.set_runtime_state(RuntimeState {
            status: OperationalStatus::Operational,
            reason: None,
            action,
        });
    }

    fn set_provisioning_action(&mut self, action: Option<RuntimeAction>) {
        self.set_runtime_state(RuntimeState {
            status: OperationalStatus::Provisioning,
            reason: None,
            action,
        });
    }

    fn set_degraded(&mut self, reason: DegradedReason, action: Option<RuntimeAction>) {
        self.set_runtime_state(RuntimeState {
            status: OperationalStatus::Degraded,
            reason: Some(reason),
            action,
        });
    }

    fn reason_for_current_action(&self) -> DegradedReason {
        match self.state.action {
            Some(RuntimeAction::WifiConnecting) => DegradedReason::WifiDisconnected,
            Some(RuntimeAction::MqttConnecting) => DegradedReason::MqttDisconnected,
            None => DegradedReason::Unknown,
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
        resources: &mut ResourceConfig,
        module_manager: &mut ModuleManager,
        device_registry: &mut DeviceRegistry,
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
                device_registry,
            )?;

            let changed_modules = module_manager.poll_changes()?;
            if let Some(mqtt) = mqtt.as_mut() {
                module_manager.publish_states_for_modules(
                    mqtt,
                    contract.topics(),
                    &changed_modules,
                )?;
                device_registry.publish_states_for_modules(
                    mqtt,
                    contract.topics(),
                    module_manager,
                    &changed_modules,
                )?;
            }

            while let Ok(action) = command_rx.try_recv() {
                if let Some(mqtt) = mqtt.as_mut() {
                    self.handle_runtime_command(
                        mqtt,
                        contract,
                        resources,
                        module_manager,
                        device_registry,
                        action,
                    )?;
                }
            }

            self.sleep_with_indicator(Duration::from_millis(200));
        }
    }

    fn check_runtime_connectivity(
        &mut self,
        wifi: &mut esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
        mqtt: &mut Option<mqtt::MqttManager>,
        mqtt_config: &crate::config::types::MqttConfig,
        contract: &MqttContract,
        command_tx: &mpsc::Sender<RuntimeCommand>,
        resources: &ResourceConfig,
        module_manager: &mut ModuleManager,
        device_registry: &DeviceRegistry,
    ) -> Result<(), AppError> {
        if !wifi.is_connected()? || !wifi.is_up()? {
            self.set_degraded(
                DegradedReason::WifiDisconnected,
                Some(RuntimeAction::WifiConnecting),
            );
            let now = Instant::now();
            let retry_at = *self
                .wifi_retry_at
                .get_or_insert_with(|| now + Duration::from_secs(2));

            if now < retry_at {
                return Ok(());
            }

            // Stop MQTT retries while the STA link is down.
            let _ = mqtt.take();
            self.set_degraded(
                DegradedReason::WifiDisconnected,
                Some(RuntimeAction::WifiConnecting),
            );

            if let Err(error) = wifi.connect().and_then(|_| wifi.wait_netif_up()) {
                warn!("Wi-Fi reconnect attempt failed: {error}");
                self.set_degraded(
                    DegradedReason::WifiDisconnected,
                    Some(RuntimeAction::WifiConnecting),
                );
                self.wifi_retry_at = Some(Instant::now() + Duration::from_secs(2));
                return Ok(());
            }

            self.wifi_retry_at = None;
            self.mqtt_retry_at = None;

            self.set_degraded(
                DegradedReason::MqttDisconnected,
                Some(RuntimeAction::MqttConnecting),
            );
            match self.establish_mqtt_session(
                mqtt_config,
                contract,
                resources,
                module_manager,
                device_registry,
                command_tx,
                wifi,
            ) {
                Ok(session) => {
                    *mqtt = Some(session);
                    self.set_operational_action(None);
                }
                Err(error) => {
                    warn!("MQTT reconnect after Wi-Fi recovery failed: {error}");
                    self.set_degraded(
                        classify_wifi_first(wifi)?,
                        Some(RuntimeAction::MqttConnecting),
                    );
                    self.mqtt_retry_at = Some(Instant::now() + Duration::from_secs(2));
                }
            }

            return Ok(());
        }

        if mqtt
            .as_ref()
            .is_some_and(|mqtt| mqtt.is_connected().ok() == Some(false))
        {
            self.set_degraded(
                DegradedReason::MqttDisconnected,
                Some(RuntimeAction::MqttConnecting),
            );
            let now = Instant::now();
            let retry_at = *self
                .mqtt_retry_at
                .get_or_insert_with(|| now + Duration::from_secs(2));

            if now < retry_at {
                return Ok(());
            }

            self.set_degraded(
                DegradedReason::MqttDisconnected,
                Some(RuntimeAction::MqttConnecting),
            );

            let Some(active_mqtt) = mqtt.as_mut() else {
                return Ok(());
            };

            if let Err(error) = active_mqtt.wait_until_connected(Duration::from_secs(15)) {
                let reason = classify_wifi_first(wifi)?;
                warn!("MQTT reconnect attempt failed: {error}");
                self.set_degraded(reason, Some(RuntimeAction::MqttConnecting));
                self.mqtt_retry_at = Some(Instant::now() + Duration::from_secs(2));
                return Ok(());
            }
            self.mqtt_retry_at = None;
            contract.publish_birth(
                active_mqtt,
                &self.gpio_manager.snapshot(resources, &self.schema_registry),
                &device_registry.snapshot(resources),
                &self.schema_registry.module_type_snapshots(),
                &device_registry.type_schemas(&self.schema_registry),
            )?;
            module_manager.publish_initial_states(active_mqtt, contract.topics())?;
            device_registry.publish_initial_states(
                active_mqtt,
                contract.topics(),
                module_manager,
            )?;
            self.set_operational_action(None);
        }

        if self.state.status == OperationalStatus::Degraded
            && self.state.reason == Some(DegradedReason::MqttDisconnected)
            && self.state.action == Some(RuntimeAction::MqttConnecting)
            && mqtt
                .as_ref()
                .is_some_and(|mqtt| mqtt.is_connected().ok() == Some(true))
        {
            self.set_operational_action(None);
        }

        Ok(())
    }

    fn handle_runtime_command(
        &mut self,
        mqtt: &mut mqtt::MqttManager,
        contract: &MqttContract,
        resources: &mut ResourceConfig,
        module_manager: &mut ModuleManager,
        device_registry: &mut DeviceRegistry,
        action: RuntimeCommand,
    ) -> Result<(), AppError> {
        match action {
            RuntimeCommand::Contract(action) => self.handle_contract_action(
                mqtt,
                contract,
                resources,
                module_manager,
                device_registry,
                action,
            ),
            RuntimeCommand::Device(command) => {
                if let Err(error) =
                    device_registry.handle_command(mqtt, contract.topics(), module_manager, command)
                {
                    warn!("Device command failed: {error}");
                }
                Ok(())
            }
            RuntimeCommand::Module(command) => {
                let command_text = String::from_utf8(command.payload.clone())
                    .map_err(AppError::from)?
                    .trim()
                    .to_uppercase();
                match module_manager.execute_command(&command.module_id, &command_text) {
                    Ok(changed_modules) => {
                        module_manager.publish_states_for_modules(
                            mqtt,
                            contract.topics(),
                            &changed_modules,
                        )?;
                        device_registry.publish_states_for_modules(
                            mqtt,
                            contract.topics(),
                            module_manager,
                            &changed_modules,
                        )?;
                    }
                    Err(error) => warn!("Module command failed: {error}"),
                }
                Ok(())
            }
        }
    }

    fn handle_contract_action(
        &mut self,
        mqtt: &mut mqtt::MqttManager,
        contract: &MqttContract,
        resources: &mut ResourceConfig,
        module_manager: &mut ModuleManager,
        device_registry: &mut DeviceRegistry,
        action: ContractAction,
    ) -> Result<(), AppError> {
        match action {
            ContractAction::GetConfig { request_id } => {
                let snapshot = self.gpio_manager.snapshot(resources, &self.schema_registry);
                contract.publish_birth(
                    mqtt,
                    &snapshot,
                    &device_registry.snapshot(resources),
                    &self.schema_registry.module_type_snapshots(),
                    &device_registry.type_schemas(&self.schema_registry),
                )?;
                contract.publish_config_result(
                    mqtt,
                    &MqttContract::ok_result(
                        "get_config",
                        request_id,
                        "Published current board, resource, and device configuration.",
                    ),
                )?;
            }
            ContractAction::ValidateResources {
                request_id,
                resources: proposed_resources,
            } => {
                let (proposed_resources, _) =
                    DeviceRegistry::normalize_config(&proposed_resources, &self.schema_registry)?;
                let result = match self
                    .gpio_manager
                    .validate_config(&proposed_resources, &self.schema_registry)
                    .and_then(|_| {
                        DeviceRegistry::validate_config(&proposed_resources, &self.schema_registry)
                    }) {
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
            } => {
                let (proposed_resources, _) =
                    DeviceRegistry::normalize_config(&proposed_resources, &self.schema_registry)?;
                match self
                    .gpio_manager
                    .validate_config(&proposed_resources, &self.schema_registry)
                    .and_then(|_| {
                        DeviceRegistry::validate_config(&proposed_resources, &self.schema_registry)
                    }) {
                    Ok(()) => {
                        let reloaded_modules = self.load_runtime_modules(&proposed_resources)?;
                        let reloaded_devices =
                            DeviceRegistry::load(&proposed_resources, &self.schema_registry)?;
                        self.store.save_resources(&proposed_resources)?;
                        *resources = proposed_resources;
                        *module_manager = reloaded_modules;
                        *device_registry = reloaded_devices;

                        let snapshot = self.gpio_manager.snapshot(resources, &self.schema_registry);
                        contract.publish_resources_snapshot(mqtt, &snapshot)?;
                        contract.publish_birth(
                            mqtt,
                            &snapshot,
                            &device_registry.snapshot(resources),
                            &self.schema_registry.module_type_snapshots(),
                            &device_registry.type_schemas(&self.schema_registry),
                        )?;
                        module_manager.publish_initial_states(mqtt, contract.topics())?;
                        device_registry.publish_initial_states(
                            mqtt,
                            contract.topics(),
                            module_manager,
                        )?;
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
                            &MqttContract::error_result(
                                "set_resources",
                                request_id,
                                error.to_string(),
                            ),
                        )?;
                    }
                }
            }
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

    fn load_effective_resources(&self) -> Result<ResourceConfig, AppError> {
        let stored = self.store.load_resources()?;
        let (normalized, changed) =
            DeviceRegistry::normalize_config(&stored, &self.schema_registry)?;

        if changed {
            info!("Auto-provisioned logical devices for single-device module instances");
            self.store.save_resources(&normalized)?;
        }

        Ok(normalized)
    }

    fn sleep_with_indicator(&mut self, duration: Duration) {
        let started_at = Instant::now();

        while started_at.elapsed() < duration {
            self.led_indicator.tick();
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn load_runtime_modules(
        &mut self,
        resources: &ResourceConfig,
    ) -> Result<ModuleManager, AppError> {
        let mut gpio_manager = GpioManager::new(BoardProfile::esp32_devkit_v1());
        let module_manager =
            ModuleManager::load(resources, &mut gpio_manager, &self.schema_registry)?;
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
                    self.set_degraded(
                        DegradedReason::WifiDisconnected,
                        Some(RuntimeAction::WifiConnecting),
                    );
                    self.sleep_with_indicator(Duration::from_secs(2));
                }
            }
        }
    }

    fn establish_mqtt_session(
        &mut self,
        mqtt_config: &crate::config::types::MqttConfig,
        contract: &MqttContract,
        resources: &ResourceConfig,
        module_manager: &mut ModuleManager,
        device_registry: &DeviceRegistry,
        command_tx: &mpsc::Sender<RuntimeCommand>,
        wifi: &esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
    ) -> Result<mqtt::MqttManager, AppError> {
        let last_will = contract.last_will()?;
        let command_topic = contract.topics().command_wildcard();
        let device_command_topic = contract.topics().device_command_wildcard();
        let module_command_topic = contract.topics().module_command_wildcard();
        let mut mqtt = mqtt::MqttManager::connect_with_last_will(mqtt_config, Some(&last_will))?;

        if let Err(error) = mqtt.wait_until_connected(Duration::from_secs(15)) {
            self.set_degraded(
                classify_wifi_first(wifi)?,
                Some(RuntimeAction::MqttConnecting),
            );
            return Err(error);
        }

        mqtt.subscribe(command_topic.as_str(), QoS::AtMostOnce, {
            let command_tx = command_tx.clone();
            let contract = contract.clone();

            move |message| {
                let _ = command_tx.send(RuntimeCommand::Contract(contract.parse_action(message)));
            }
        })?;
        mqtt.subscribe(device_command_topic.as_str(), QoS::AtMostOnce, {
            let command_tx = command_tx.clone();
            let topics = contract.topics().clone();

            move |message| {
                if let Some(device_id) = topics.parse_device_command_topic(&message.topic) {
                    let _ = command_tx.send(RuntimeCommand::Device(DeviceCommand {
                        device_id,
                        payload: message.payload.clone(),
                    }));
                }
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
        contract.publish_birth(
            &mut mqtt,
            &self.gpio_manager.snapshot(resources, &self.schema_registry),
            &device_registry.snapshot(resources),
            &self.schema_registry.module_type_snapshots(),
            &device_registry.type_schemas(&self.schema_registry),
        )?;
        module_manager.publish_initial_states(&mut mqtt, contract.topics())?;
        device_registry.publish_initial_states(&mut mqtt, contract.topics(), module_manager)?;

        Ok(mqtt)
    }
}

fn take_peripherals() -> Result<Peripherals, AppError> {
    Peripherals::take()
        .map_err(|e| AppError::Message(format!("Failed to take ESP peripherals: {e:?}")))
}

fn classify_wifi_first(
    wifi: &esp_idf_svc::wifi::BlockingWifi<esp_idf_svc::wifi::EspWifi<'static>>,
) -> Result<DegradedReason, AppError> {
    if !wifi.is_connected()? || !wifi.is_up()? {
        Ok(DegradedReason::WifiDisconnected)
    } else {
        Ok(DegradedReason::MqttDisconnected)
    }
}
