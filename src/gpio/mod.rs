use std::collections::{HashMap, HashSet};

use log::info;
use serde::Serialize;

use crate::board::{BoardProfile, BoardProfileSnapshot};
use crate::config::types::{
    ModuleInstanceConfig, ModuleSettings, PinBindingConfig, ResourceConfig, ResourceUsage,
};
use crate::error::AppError;
use crate::schemas::{validate, SchemaRegistry};

#[derive(Debug, Clone, Serialize)]
pub struct ResourceConfigSnapshot {
    pub version: u32,
    pub board: BoardProfileSnapshot,
    pub module_instances: Vec<ModuleInstanceSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleInstanceSnapshot {
    pub id: String,
    pub module_type_id: String,
    pub display_name: Option<String>,
    pub settings: ModuleSettings,
    pub bindings: Vec<PinBindingSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PinBindingSnapshot {
    pub role_id: String,
    pub pin: u8,
    pub usage: ResourceUsage,
}

#[derive(Debug, Clone)]
pub struct ClaimedModulePins {
    module_id: String,
    pins_by_role_id: HashMap<String, u8>,
}

#[derive(Debug, Clone)]
struct RuntimeClaim {
    module_id: String,
    role_id: String,
}

pub struct GpioManager {
    board: &'static BoardProfile,
    claims: HashMap<u8, RuntimeClaim>,
}

impl GpioManager {
    pub fn new(board: &'static BoardProfile) -> Self {
        Self {
            board,
            claims: HashMap::new(),
        }
    }

    pub fn validate_config(
        &self,
        config: &ResourceConfig,
        schemas: &SchemaRegistry,
    ) -> Result<(), AppError> {
        let mut seen_module_ids = HashSet::new();
        let mut seen_pins = HashMap::<u8, String>::new();

        for module in &config.module_instances {
            if module.id.trim().is_empty() {
                return Err(AppError::Message(
                    "Module instance id cannot be empty".into(),
                ));
            }

            if !seen_module_ids.insert(module.id.clone()) {
                return Err(AppError::Message(format!(
                    "Module instance '{}' is defined more than once",
                    module.id
                )));
            }

            self.validate_module(module, schemas)?;

            for binding in &module.bindings {
                let pin = binding.pin()?;
                if let Some(existing_owner) = seen_pins.insert(pin, module.id.clone()) {
                    return Err(AppError::Message(format!(
                        "GPIO{pin} is assigned to both '{}' and '{}'",
                        existing_owner, module.id
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn claim_module_instance(
        &mut self,
        config: &ModuleInstanceConfig,
        schemas: &SchemaRegistry,
    ) -> Result<ClaimedModulePins, AppError> {
        self.validate_module(config, schemas)?;

        let mut claimed = HashMap::new();

        for binding in &config.bindings {
            let pin = binding.pin()?;

            if let Some(existing) = self.claims.get(&pin) {
                return Err(AppError::Message(format!(
                    "GPIO{pin} is already claimed by module '{}' role {}",
                    existing.module_id, existing.role_id
                )));
            }

            self.claims.insert(
                pin,
                RuntimeClaim {
                    module_id: config.id.clone(),
                    role_id: binding.role_id.clone(),
                },
            );
            claimed.insert(binding.role_id.clone(), pin);
        }

        info!("Claimed GPIO bindings for module '{}'", config.id);

        Ok(ClaimedModulePins {
            module_id: config.id.clone(),
            pins_by_role_id: claimed,
        })
    }

    pub fn release_module(&mut self, module_id: &str) {
        self.claims.retain(|_, claim| claim.module_id != module_id);
    }

    pub fn snapshot(
        &self,
        config: &ResourceConfig,
        schemas: &SchemaRegistry,
    ) -> ResourceConfigSnapshot {
        ResourceConfigSnapshot {
            version: config.version,
            board: self.board.snapshot(),
            module_instances: config
                .module_instances
                .iter()
                .map(|instance| ModuleInstanceSnapshot {
                    id: instance.id.clone(),
                    module_type_id: instance.module_type_id.clone(),
                    display_name: instance.display_name.clone(),
                    settings: instance.settings.clone(),
                    bindings: instance
                        .bindings
                        .iter()
                        .filter_map(|binding| {
                            binding.pin().ok().map(|pin| PinBindingSnapshot {
                                role_id: binding.role_id.clone(),
                                pin,
                                usage: schemas
                                    .lookup_binding_role(&binding.role_id)
                                    .map(|role| role.resource_usage)
                                    .unwrap_or(ResourceUsage::Output),
                            })
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn validate_module(
        &self,
        module: &ModuleInstanceConfig,
        schemas: &SchemaRegistry,
    ) -> Result<(), AppError> {
        validate::validate_module_instance(schemas, module)?;
        let mut seen_roles = HashSet::new();

        for binding in &module.bindings {
            if !seen_roles.insert(binding.role_id.as_str()) {
                return Err(AppError::Message(format!(
                    "Module '{}' defines role '{}' more than once",
                    module.id, binding.role_id
                )));
            }

            self.validate_binding(module, binding, schemas)?;
        }

        Ok(())
    }

    fn validate_binding(
        &self,
        module: &ModuleInstanceConfig,
        binding: &PinBindingConfig,
        schemas: &SchemaRegistry,
    ) -> Result<(), AppError> {
        let pin = binding.pin()?;
        let usage = schemas
            .lookup_binding_role_required(&binding.role_id)?
            .resource_usage;

        if !self.board.supports(pin, usage) {
            return Err(AppError::Message(format!(
                "GPIO{pin} does not support {:?} for module '{}' role '{}'",
                usage, module.id, binding.role_id
            )));
        }

        Ok(())
    }
}

impl ClaimedModulePins {
    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn pin_for_schema_role(&self, role_id: &str) -> Option<u8> {
        self.pins_by_role_id.get(role_id).copied()
    }
}
