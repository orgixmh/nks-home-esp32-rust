use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::config::types::{DeviceInstanceConfig, ResourceConfig};
use crate::error::AppError;
use crate::modules::ModuleManager;
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;
use crate::schemas::types::{DeviceBindingMode, DeviceTypeSchemaSnapshot};
use crate::schemas::{validate, SchemaRegistry};

use super::switch::SwitchController;
use super::traits::{DeviceCommand, DeviceController};

#[derive(Debug, Clone, Serialize)]
pub struct DeviceConfigSnapshot {
    pub device_instances: Vec<DeviceInstanceSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInstanceSnapshot {
    pub id: String,
    pub device_type_id: String,
    pub display_name: Option<String>,
    pub driver_module_id: String,
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

    pub fn normalize_config(
        config: &ResourceConfig,
        schemas: &SchemaRegistry,
    ) -> Result<(ResourceConfig, bool), AppError> {
        let mut normalized = config.clone();
        let mut existing_driver_modules = normalized
            .device_instances
            .iter()
            .map(|device| device.driver_module_id.clone())
            .collect::<HashSet<_>>();
        let mut existing_device_ids = normalized
            .device_instances
            .iter()
            .map(|device| device.id.clone())
            .collect::<HashSet<_>>();
        let mut changed = false;
        let module_instances = normalized.module_instances.clone();

        for module in &module_instances {
            let schema = schemas.lookup_module_type_required(&module.module_type_id)?;
            if !matches!(schema.device_binding_mode, DeviceBindingMode::Single) {
                continue;
            }

            if existing_driver_modules.contains(&module.id) {
                continue;
            }

            let Some(default_device_type_id) = schema.default_device_type_id else {
                continue;
            };

            let default_device_id = format!("{}__device", module.id);
            if existing_device_ids.contains(&default_device_id) {
                return Err(AppError::Message(format!(
                    "Cannot auto-provision device for module '{}' because device id '{}' is already in use",
                    module.id, default_device_id
                )));
            }

            normalized.device_instances.push(DeviceInstanceConfig {
                id: default_device_id.clone(),
                device_type_id: default_device_type_id.to_string(),
                display_name: module
                    .display_name
                    .clone()
                    .or_else(|| Some(module.id.clone())),
                driver_module_id: module.id.clone(),
            });
            existing_driver_modules.insert(module.id.clone());
            existing_device_ids.insert(default_device_id);
            changed = true;
        }

        Ok((normalized, changed))
    }

    pub fn validate_config(
        config: &ResourceConfig,
        schemas: &SchemaRegistry,
    ) -> Result<(), AppError> {
        let mut device_ids = HashSet::new();
        let mut devices_by_module = HashMap::<&str, usize>::new();

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

            *devices_by_module
                .entry(device.driver_module_id.as_str())
                .or_insert(0) += 1;

            validate::validate_device_instance(schemas, config, device)?;
        }

        for module in &config.module_instances {
            let schema = schemas.lookup_module_type_required(&module.module_type_id)?;
            let bound_devices = devices_by_module.get(module.id.as_str()).copied().unwrap_or(0);

            if matches!(schema.device_binding_mode, DeviceBindingMode::Single) && bound_devices != 1
            {
                return Err(AppError::Message(format!(
                    "Module '{}' requires exactly one logical device, found {}",
                    module.id, bound_devices
                )));
            }
        }

        Ok(())
    }

    pub fn load(config: &ResourceConfig, schemas: &SchemaRegistry) -> Result<Self, AppError> {
        Self::validate_config(config, schemas)?;

        let mut devices: HashMap<String, Box<dyn DeviceController>> = HashMap::new();
        let mut module_to_devices: HashMap<String, Vec<String>> = HashMap::new();

        for device in &config.device_instances {
            let controller = build_controller(device)?;

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
                    device_type_id: device.device_type_id.clone(),
                    display_name: device.display_name.clone(),
                    driver_module_id: device.driver_module_id.clone(),
                })
                .collect(),
        }
    }

    pub fn type_schemas(&self, schemas: &SchemaRegistry) -> Vec<DeviceTypeSchemaSnapshot> {
        schemas.device_type_snapshots()
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

fn build_controller(
    device: &DeviceInstanceConfig,
) -> Result<Box<dyn DeviceController>, AppError> {
    match device.device_type_id.as_str() {
        "core:switch" => Ok(Box::new(SwitchController::new(
            device.id.clone(),
            device.driver_module_id.clone(),
        ))),
        other => Err(AppError::Message(format!(
            "Unsupported device controller type '{}'",
            other
        ))),
    }
}
