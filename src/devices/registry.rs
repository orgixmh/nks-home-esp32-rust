use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::config::types::{DeviceType, ModuleType, ResourceConfig};
use crate::error::AppError;
use crate::modules::ModuleManager;
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;

use super::switch::SwitchController;
use super::traits::{DeviceCommand, DeviceController};

#[derive(Debug, Clone, Serialize)]
pub struct DeviceConfigSnapshot {
    pub device_instances: Vec<DeviceInstanceSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInstanceSnapshot {
    pub id: String,
    pub device_type: DeviceType,
    pub display_name: Option<String>,
    pub driver_module_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceTypeSchemaSnapshot {
    pub device_type: DeviceType,
    pub commands: Vec<String>,
    pub state_fields: Vec<String>,
    pub capabilities: Vec<String>,
}

pub struct DeviceRegistry {
    devices: HashMap<String, Box<dyn DeviceController>>,
    module_to_devices: HashMap<String, Vec<String>>,
}

impl DeviceRegistry {
    pub fn empty() -> Self {
        Self {
            devices: HashMap::new(),
            module_to_devices: HashMap::new(),
        }
    }

    pub fn validate_config(config: &ResourceConfig) -> Result<(), AppError> {
        let module_types = config
            .module_instances
            .iter()
            .map(|module| (module.id.as_str(), module.module_type))
            .collect::<HashMap<_, _>>();
        let mut device_ids = HashSet::new();
        let mut driver_modules = HashSet::new();

        for device in &config.device_instances {
            if device.id.trim().is_empty() {
                return Err(AppError::Message(
                    "Device instance id cannot be empty".into(),
                ));
            }

            if !device_ids.insert(device.id.clone()) {
                return Err(AppError::Message(format!(
                    "Device instance '{}' is defined more than once",
                    device.id
                )));
            }

            let Some(module_type) = module_types.get(device.driver_module_id.as_str()) else {
                return Err(AppError::Message(format!(
                    "Device '{}' references unknown driver module '{}'",
                    device.id, device.driver_module_id
                )));
            };

            if !driver_modules.insert(device.driver_module_id.clone()) {
                return Err(AppError::Message(format!(
                    "Driver module '{}' is assigned to more than one logical device",
                    device.driver_module_id
                )));
            }

            match (device.device_type, module_type) {
                (DeviceType::Switch, ModuleType::Switch | ModuleType::GpioOutput) => {}
            }
        }

        Ok(())
    }

    pub fn load(config: &ResourceConfig) -> Result<Self, AppError> {
        Self::validate_config(config)?;

        let mut devices: HashMap<String, Box<dyn DeviceController>> = HashMap::new();
        let mut module_to_devices: HashMap<String, Vec<String>> = HashMap::new();

        for device in &config.device_instances {
            let controller: Box<dyn DeviceController> = match device.device_type {
                DeviceType::Switch => Box::new(SwitchController::new(
                    device.id.clone(),
                    device.driver_module_id.clone(),
                )),
            };

            module_to_devices
                .entry(controller.driver_module_id().to_string())
                .or_default()
                .push(device.id.clone());
            devices.insert(device.id.clone(), controller);
        }

        Ok(Self {
            devices,
            module_to_devices,
        })
    }

    pub fn snapshot(&self, config: &ResourceConfig) -> DeviceConfigSnapshot {
        DeviceConfigSnapshot {
            device_instances: config
                .device_instances
                .iter()
                .map(|device| DeviceInstanceSnapshot {
                    id: device.id.clone(),
                    device_type: device.device_type,
                    display_name: device.display_name.clone(),
                    driver_module_id: device.driver_module_id.clone(),
                })
                .collect(),
        }
    }

    pub fn type_schemas(&self) -> Vec<DeviceTypeSchemaSnapshot> {
        vec![DeviceTypeSchemaSnapshot {
            device_type: DeviceType::Switch,
            commands: vec!["ON".into(), "OFF".into(), "TOGGLE".into()],
            state_fields: vec!["state".into()],
            capabilities: vec!["binary_output".into(), "retained_state".into()],
        }]
    }

    pub fn publish_initial_states(
        &self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
        modules: &ModuleManager,
    ) -> Result<(), AppError> {
        for device in self.devices.values() {
            device.publish_state(mqtt, topics, modules)?;
        }

        Ok(())
    }

    pub fn handle_command(
        &self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
        modules: &mut ModuleManager,
        command: DeviceCommand,
    ) -> Result<(), AppError> {
        let device = self
            .devices
            .get(&command.device_id)
            .ok_or_else(|| AppError::Message(format!("Unknown device '{}'", command.device_id)))?;
        let changed_modules = device.handle_command(&command.payload, modules)?;
        self.publish_states_for_modules(mqtt, topics, modules, &changed_modules)
    }

    pub fn publish_states_for_modules(
        &self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
        modules: &ModuleManager,
        changed_modules: &[String],
    ) -> Result<(), AppError> {
        let mut published = HashSet::new();

        for module_id in changed_modules {
            if let Some(device_ids) = self.module_to_devices.get(module_id) {
                for device_id in device_ids {
                    if !published.insert(device_id.clone()) {
                        continue;
                    }

                    if let Some(device) = self.devices.get(device_id) {
                        device.publish_state(mqtt, topics, modules)?;
                    }
                }
            }
        }

        Ok(())
    }
}
