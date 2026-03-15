use std::time::{Duration, Instant};

use esp_idf_svc::hal::gpio::{AnyIOPin, Input, Output, PinDriver};
use esp_idf_svc::mqtt::client::QoS;
use log::info;
use serde::Serialize;

use crate::config::types::ModuleSettings;
use crate::error::AppError;
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;

use super::events::{ModuleEvent, ModuleEventKind, ModuleExecution};
use super::traits::Module;
use super::DriverBinaryState;

const TRIGGER_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Serialize)]
struct RelayStatePayload {
    state: DriverBinaryState,
}

pub struct RelayGpioModule {
    id: String,
    settings: ModuleSettings,
    output: PinDriver<'static, AnyIOPin, Output>,
    trigger_input: Option<DebouncedToggleInput>,
    state: DriverBinaryState,
    auto_off_deadline: Option<Instant>,
}

struct DebouncedToggleInput {
    input: PinDriver<'static, AnyIOPin, Input>,
    stable_level: bool,
    last_raw_level: bool,
    last_raw_change_at: Instant,
}

enum RelayStateChangeSource<'a> {
    MqttCommand(&'a str),
    GpioTrigger,
    AutoOffTimer,
    ExternalModule,
}

impl RelayGpioModule {
    pub fn new(
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
            state: DriverBinaryState::Off,
            auto_off_deadline: None,
        })
    }

    fn apply_target_state(
        &mut self,
        next: DriverBinaryState,
        source: RelayStateChangeSource<'_>,
        now: Instant,
    ) -> Result<ModuleExecution, AppError> {
        match next {
            DriverBinaryState::On => self.output.set_high()?,
            DriverBinaryState::Off => self.output.set_low()?,
        }

        let state_changed = self.state != next;
        self.state = next;

        self.auto_off_deadline = match next {
            DriverBinaryState::On => self
                .settings
                .auto_off_ms
                .map(|timeout| now + Duration::from_millis(timeout)),
            DriverBinaryState::Off => None,
        };

        if !state_changed {
            if matches!(source, RelayStateChangeSource::MqttCommand("ON"))
                && next == DriverBinaryState::On
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
        source: RelayStateChangeSource<'_>,
        now: Instant,
    ) -> Result<ModuleExecution, AppError> {
        let next = match self.state {
            DriverBinaryState::On => DriverBinaryState::Off,
            DriverBinaryState::Off => DriverBinaryState::On,
        };

        self.apply_target_state(next, source, now)
    }
}

impl Module for RelayGpioModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn binary_state(&self) -> Option<DriverBinaryState> {
        Some(self.state)
    }

    fn publish_state(
        &mut self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
    ) -> Result<(), AppError> {
        mqtt.publish_json(
            &topics.module_state(self.id()),
            &RelayStatePayload { state: self.state },
            QoS::AtLeastOnce,
            true,
        )?;

        Ok(())
    }

    fn handle_command(&mut self, command: &str, now: Instant) -> Result<ModuleExecution, AppError> {
        match command {
            "ON" => self.apply_target_state(
                DriverBinaryState::On,
                RelayStateChangeSource::MqttCommand("ON"),
                now,
            ),
            "OFF" => self.apply_target_state(
                DriverBinaryState::Off,
                RelayStateChangeSource::MqttCommand("OFF"),
                now,
            ),
            "TOGGLE" => self.toggle(RelayStateChangeSource::MqttCommand("TOGGLE"), now),
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
                state: DriverBinaryState::On,
            } => {
                if self
                    .settings
                    .external_on_triggers
                    .iter()
                    .any(|trigger| trigger.source_module_id == event.source_module_id)
                {
                    return self.toggle(RelayStateChangeSource::ExternalModule, now);
                }
            }
            ModuleEventKind::BinaryStateChanged {
                state: DriverBinaryState::Off,
            } => {}
        }

        Ok(ModuleExecution::default())
    }

    fn poll(&mut self, now: Instant) -> Result<ModuleExecution, AppError> {
        if let Some(deadline) = self.auto_off_deadline {
            if now >= deadline {
                return self.apply_target_state(
                    DriverBinaryState::Off,
                    RelayStateChangeSource::AutoOffTimer,
                    now,
                );
            }
        }

        if let Some(trigger_input) = &mut self.trigger_input {
            if trigger_input.poll(now) {
                return self.toggle(RelayStateChangeSource::GpioTrigger, now);
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

impl RelayStateChangeSource<'_> {
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
