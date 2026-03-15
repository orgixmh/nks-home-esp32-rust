use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::config::types::{ModuleRole, ModuleType, ResourceConfig};
use crate::error::AppError;
use crate::gpio::GpioManager;
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;

use super::events::ModuleExecution;
use super::relay_gpio::RelayGpioModule;
use super::traits::Module;
use super::{DriverBinaryState, ModuleCommand};

pub struct ModuleManager {
    modules: HashMap<String, Box<dyn Module>>,
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
                        Box::new(RelayGpioModule::new(
                            instance.id.clone(),
                            instance.settings.clone(),
                            output_pin,
                            trigger_pin,
                        )?),
                    );
                }
            }
        }

        log::info!("Loaded {} runtime module instance(s)", modules.len());

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
        let changed = self.execute_command(&command.module_id, &command_text)?;

        self.publish_states_for_modules(mqtt, topics, &changed)
    }

    pub fn execute_command(
        &mut self,
        module_id: &str,
        command: &str,
    ) -> Result<Vec<String>, AppError> {
        let now = Instant::now();
        let execution = {
            let module = self
                .modules
                .get_mut(module_id)
                .ok_or_else(|| AppError::Message(format!("Unknown module '{module_id}'")))?;

            module.handle_command(command, now)?
        };

        self.collect_changed_modules(module_id.to_string(), execution, now)
    }

    pub fn binary_state(&self, module_id: &str) -> Option<DriverBinaryState> {
        self.modules
            .get(module_id)
            .and_then(|module| module.binary_state())
    }

    pub fn poll_changes(&mut self) -> Result<Vec<String>, AppError> {
        let now = Instant::now();
        let module_ids = self.modules.keys().cloned().collect::<Vec<_>>();
        let mut changed = Vec::new();

        for module_id in module_ids {
            let execution = {
                let module = self
                    .modules
                    .get_mut(&module_id)
                    .ok_or_else(|| AppError::Message(format!("Unknown module '{}'", module_id)))?;

                module.poll(now)?
            };

            changed.extend(self.collect_changed_modules(module_id, execution, now)?);
        }

        Ok(changed)
    }

    pub fn publish_states_for_modules(
        &mut self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
        module_ids: &[String],
    ) -> Result<(), AppError> {
        for module_id in module_ids {
            let module = self
                .modules
                .get_mut(module_id)
                .ok_or_else(|| AppError::Message(format!("Unknown module '{module_id}'")))?;
            module.publish_state(mqtt, topics)?;
        }

        Ok(())
    }

    fn collect_changed_modules(
        &mut self,
        source_module_id: String,
        execution: ModuleExecution,
        now: Instant,
    ) -> Result<Vec<String>, AppError> {
        let mut queue = VecDeque::new();
        let mut changed = Vec::new();

        if execution.state_changed {
            changed.push(source_module_id.clone());
        }
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

                if execution.state_changed {
                    changed.push(module_id.clone());
                }
                queue.extend(execution.events);
            }
        }

        Ok(changed)
    }
}

fn output_role_for(module_type: ModuleType) -> ModuleRole {
    match module_type {
        ModuleType::Switch => ModuleRole::RelayOutput,
        ModuleType::GpioOutput => ModuleRole::Output,
    }
}
