use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use esp_idf_svc::hal::gpio::{AnyIOPin, Input, Output, PinDriver};
use esp_idf_svc::mqtt::client::QoS;
use log::{info, warn};
use serde::Serialize;

use crate::config::types::{
    ModuleInstanceConfig, ModuleRole, ModuleSettings, ModuleType, ResourceConfig,
};
use crate::error::AppError;
use crate::gpio::GpioManager;
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;

const TRIGGER_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone)]
pub struct ModuleCommand {
    pub module_id: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum BinaryState {
    On,
    Off,
}

#[derive(Debug, Clone)]
struct ModuleEvent {
    source_module_id: String,
    kind: ModuleEventKind,
}

#[derive(Debug, Clone)]
enum ModuleEventKind {
    BinaryStateChanged { state: BinaryState },
}

#[derive(Debug, Default)]
struct ModuleExecution {
    state_changed: bool,
    events: Vec<ModuleEvent>,
}

pub trait Module {
    fn id(&self) -> &str;
    fn publish_state(
        &mut self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
    ) -> Result<(), AppError>;
    fn handle_command(&mut self, command: &str, now: Instant) -> Result<ModuleExecution, AppError>;
    fn handle_event(
        &mut self,
        event: &ModuleEvent,
        now: Instant,
    ) -> Result<ModuleExecution, AppError>;
    fn poll(&mut self, now: Instant) -> Result<ModuleExecution, AppError>;
}

pub struct ModuleManager {
    modules: HashMap<String, Box<dyn Module>>,
}

#[derive(Debug, Clone, Serialize)]
struct SwitchStatePayload {
    state: BinaryState,
}

struct SwitchModule {
    id: String,
    settings: ModuleSettings,
    output: PinDriver<'static, AnyIOPin, Output>,
    trigger_input: Option<DebouncedToggleInput>,
    state: BinaryState,
    auto_off_deadline: Option<Instant>,
}

struct DebouncedToggleInput {
    input: PinDriver<'static, AnyIOPin, Input>,
    stable_level: bool,
    last_raw_level: bool,
    last_raw_change_at: Instant,
}

enum SwitchStateChangeSource<'a> {
    MqttCommand(&'a str),
    GpioTrigger,
    AutoOffTimer,
    ExternalModule,
}

impl ModuleManager {
    pub fn empty() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    pub fn load(config: &ResourceConfig, gpio_manager: &mut GpioManager) -> Result<Self, AppError> {
        let mut modules: HashMap<String, Box<dyn Module>> = HashMap::new();

        for instance in &config.module_instances {
            match instance.module_type {
                ModuleType::Switch | ModuleType::GpioOutput => {
                    let claimed = gpio_manager.claim_module_instance(instance)?;
                    let output_role = output_role_for(instance.module_type);
                    let output_pin = claimed.pin_for(output_role).ok_or_else(|| {
                        AppError::Message(format!(
                            "Module '{}' is missing claimed output pin",
                            claimed.module_id()
                        ))
                    })?;
                    let trigger_pin = claimed.pin_for(ModuleRole::WallTriggerInput);

                    modules.insert(
                        instance.id.clone(),
                        Box::new(SwitchModule::new(
                            instance.id.clone(),
                            instance.settings.clone(),
                            output_pin,
                            trigger_pin,
                        )?),
                    );
                }
            }
        }

        info!("Loaded {} runtime module instance(s)", modules.len());

        Ok(Self { modules })
    }

    pub fn publish_initial_states(
        &mut self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
    ) -> Result<(), AppError> {
        for module in self.modules.values_mut() {
            module.publish_state(mqtt, topics)?;
        }

        Ok(())
    }

    pub fn handle_command(
        &mut self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
        command: ModuleCommand,
    ) -> Result<(), AppError> {
        let command_text = String::from_utf8(command.payload)?.trim().to_uppercase();
        let now = Instant::now();
        let execution = {
            let module = self.modules.get_mut(&command.module_id).ok_or_else(|| {
                AppError::Message(format!("Unknown module '{}'", command.module_id))
            })?;

            module.handle_command(&command_text, now)?
        };

        let mut mqtt_ref = Some(mqtt);
        self.process_execution(command.module_id, execution, &mut mqtt_ref, topics, now)
    }

    pub fn poll(
        &mut self,
        mqtt: Option<&mut MqttManager>,
        topics: &MqttTopics,
    ) -> Result<(), AppError> {
        let now = Instant::now();
        let module_ids = self.modules.keys().cloned().collect::<Vec<_>>();
        let mut mqtt = mqtt;

        for module_id in module_ids {
            let execution = {
                let module = self
                    .modules
                    .get_mut(&module_id)
                    .ok_or_else(|| AppError::Message(format!("Unknown module '{}'", module_id)))?;

                module.poll(now)?
            };

            self.process_execution(module_id, execution, &mut mqtt, topics, now)?;
        }

        Ok(())
    }

    fn process_execution(
        &mut self,
        source_module_id: String,
        execution: ModuleExecution,
        mqtt: &mut Option<&mut MqttManager>,
        topics: &MqttTopics,
        now: Instant,
    ) -> Result<(), AppError> {
        let mut queue = VecDeque::new();
        self.publish_state_if_needed(&source_module_id, execution.state_changed, mqtt, topics)?;
        queue.extend(execution.events);

        while let Some(event) = queue.pop_front() {
            let module_ids = self.modules.keys().cloned().collect::<Vec<_>>();

            for module_id in module_ids {
                if module_id == event.source_module_id {
                    continue;
                }

                let execution = {
                    let module = self.modules.get_mut(&module_id).ok_or_else(|| {
                        AppError::Message(format!("Unknown module '{}'", module_id))
                    })?;

                    module.handle_event(&event, now)?
                };

                self.publish_state_if_needed(&module_id, execution.state_changed, mqtt, topics)?;
                queue.extend(execution.events);
            }
        }

        Ok(())
    }

    fn publish_state_if_needed(
        &mut self,
        module_id: &str,
        state_changed: bool,
        mqtt: &mut Option<&mut MqttManager>,
        topics: &MqttTopics,
    ) -> Result<(), AppError> {
        if !state_changed {
            return Ok(());
        }

        let Some(client) = mqtt.as_mut() else {
            return Ok(());
        };
        let module = self
            .modules
            .get_mut(module_id)
            .ok_or_else(|| AppError::Message(format!("Unknown module '{module_id}'")))?;

        module.publish_state(*client, topics)
    }
}

impl SwitchModule {
    fn new(
        id: String,
        settings: ModuleSettings,
        output_pin: u8,
        trigger_pin: Option<u8>,
    ) -> Result<Self, AppError> {
        let gpio = unsafe { AnyIOPin::new(output_pin as i32) };
        let mut output = PinDriver::output(gpio)?;
        output.set_low()?;

        let trigger_input = match trigger_pin {
            Some(pin) => Some(DebouncedToggleInput::new(pin)?),
            None => None,
        };

        Ok(Self {
            id,
            settings,
            output,
            trigger_input,
            state: BinaryState::Off,
            auto_off_deadline: None,
        })
    }

    fn apply_target_state(
        &mut self,
        next: BinaryState,
        source: SwitchStateChangeSource<'_>,
        now: Instant,
    ) -> Result<ModuleExecution, AppError> {
        match next {
            BinaryState::On => self.output.set_high()?,
            BinaryState::Off => self.output.set_low()?,
        }

        let state_changed = self.state != next;
        self.state = next;

        self.auto_off_deadline = match next {
            BinaryState::On => self
                .settings
                .auto_off_ms
                .map(|timeout| now + Duration::from_millis(timeout)),
            BinaryState::Off => None,
        };

        if !state_changed {
            if matches!(source, SwitchStateChangeSource::MqttCommand("ON"))
                && next == BinaryState::On
            {
                info!("Restarted auto-off timer for module '{}'", self.id);
            }
            return Ok(ModuleExecution::default());
        }

        info!(
            "Module '{}' changed to {:?} via {}",
            self.id,
            next,
            source.label()
        );

        Ok(ModuleExecution {
            state_changed: true,
            events: vec![ModuleEvent {
                source_module_id: self.id.clone(),
                kind: ModuleEventKind::BinaryStateChanged { state: next },
            }],
        })
    }

    fn toggle(
        &mut self,
        source: SwitchStateChangeSource<'_>,
        now: Instant,
    ) -> Result<ModuleExecution, AppError> {
        let next = match self.state {
            BinaryState::On => BinaryState::Off,
            BinaryState::Off => BinaryState::On,
        };

        self.apply_target_state(next, source, now)
    }
}

impl Module for SwitchModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn publish_state(
        &mut self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
    ) -> Result<(), AppError> {
        mqtt.publish_json(
            &topics.module_state(self.id()),
            &SwitchStatePayload { state: self.state },
            QoS::AtLeastOnce,
            true,
        )?;

        Ok(())
    }

    fn handle_command(&mut self, command: &str, now: Instant) -> Result<ModuleExecution, AppError> {
        match command {
            "ON" => self.apply_target_state(
                BinaryState::On,
                SwitchStateChangeSource::MqttCommand("ON"),
                now,
            ),
            "OFF" => self.apply_target_state(
                BinaryState::Off,
                SwitchStateChangeSource::MqttCommand("OFF"),
                now,
            ),
            "TOGGLE" => self.toggle(SwitchStateChangeSource::MqttCommand("TOGGLE"), now),
            _ => Err(AppError::Message(format!(
                "Unsupported command '{}' for module '{}'",
                command, self.id
            ))),
        }
    }

    fn handle_event(
        &mut self,
        event: &ModuleEvent,
        now: Instant,
    ) -> Result<ModuleExecution, AppError> {
        match &event.kind {
            ModuleEventKind::BinaryStateChanged {
                state: BinaryState::On,
            } => {
                if self
                    .settings
                    .external_on_triggers
                    .iter()
                    .any(|trigger| trigger.source_module_id == event.source_module_id)
                {
                    return self.toggle(SwitchStateChangeSource::ExternalModule, now);
                }
            }
            ModuleEventKind::BinaryStateChanged {
                state: BinaryState::Off,
            } => {}
        }

        Ok(ModuleExecution::default())
    }

    fn poll(&mut self, now: Instant) -> Result<ModuleExecution, AppError> {
        if let Some(deadline) = self.auto_off_deadline {
            if now >= deadline {
                return self.apply_target_state(
                    BinaryState::Off,
                    SwitchStateChangeSource::AutoOffTimer,
                    now,
                );
            }
        }

        if let Some(trigger_input) = &mut self.trigger_input {
            if trigger_input.poll(now) {
                return self.toggle(SwitchStateChangeSource::GpioTrigger, now);
            }
        }

        Ok(ModuleExecution::default())
    }
}

impl DebouncedToggleInput {
    fn new(pin: u8) -> Result<Self, AppError> {
        let gpio = unsafe { AnyIOPin::new(pin as i32) };
        let input = PinDriver::input(gpio)?;
        let level = input.is_high();
        let now = Instant::now();

        Ok(Self {
            input,
            stable_level: level,
            last_raw_level: level,
            last_raw_change_at: now,
        })
    }

    fn poll(&mut self, now: Instant) -> bool {
        let raw_level = self.input.is_high();

        if raw_level != self.last_raw_level {
            self.last_raw_level = raw_level;
            self.last_raw_change_at = now;
            return false;
        }

        if raw_level != self.stable_level
            && now.duration_since(self.last_raw_change_at) >= TRIGGER_DEBOUNCE
        {
            self.stable_level = raw_level;
            return true;
        }

        false
    }
}

impl SwitchStateChangeSource<'_> {
    fn label(&self) -> &'static str {
        match self {
            Self::MqttCommand("ON") => "mqtt_on",
            Self::MqttCommand("OFF") => "mqtt_off",
            Self::MqttCommand("TOGGLE") => "mqtt_toggle",
            Self::MqttCommand(_) => "mqtt",
            Self::GpioTrigger => "gpio_trigger",
            Self::AutoOffTimer => "auto_off",
            Self::ExternalModule => "module_trigger",
        }
    }
}

fn output_role_for(module_type: ModuleType) -> ModuleRole {
    match module_type {
        ModuleType::Switch => ModuleRole::RelayOutput,
        ModuleType::GpioOutput => ModuleRole::Output,
    }
}
