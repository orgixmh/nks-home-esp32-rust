use std::collections::{HashMap, HashSet};

use log::info;
use serde::Serialize;

use crate::board::{BoardProfile, BoardProfileSnapshot};
use crate::config::types::{
    ModuleInstanceConfig, ModuleRole, ModuleType, PinBindingConfig, ResourceBindingTarget,
    ResourceConfig, ResourceUsage,
};
use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct ResourceConfigSnapshot {
    pub board: BoardProfileSnapshot,
    pub module_instances: Vec<ModuleInstanceSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleInstanceSnapshot {
    pub id: String,
    pub module_type: ModuleType,
    pub display_name: Option<String>,
    pub bindings: Vec<PinBindingSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PinBindingSnapshot {
    pub role: ModuleRole,
    pub pin: u8,
    pub usage: ResourceUsage,
}

#[derive(Debug, Clone)]
pub struct ClaimedModulePins {
    module_id: String,
    pins_by_role: HashMap<ModuleRole, u8>,
}

#[derive(Debug, Clone)]
struct RuntimeClaim {
    module_id: String,
    module_type: ModuleType,
    role: ModuleRole,
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

    pub fn validate_config(&self, config: &ResourceConfig) -> Result<(), AppError> {
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

            self.validate_module(module)?;

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
    ) -> Result<ClaimedModulePins, AppError> {
        self.validate_module(config)?;

        let mut claimed = HashMap::new();

        for binding in &config.bindings {
            let pin = binding.pin()?;

            if let Some(existing) = self.claims.get(&pin) {
                return Err(AppError::Message(format!(
                    "GPIO{pin} is already claimed by module '{}' role {:?}",
                    existing.module_id, existing.role
                )));
            }

            self.claims.insert(
                pin,
                RuntimeClaim {
                    module_id: config.id.clone(),
                    module_type: config.module_type,
                    role: binding.role,
                },
            );
            claimed.insert(binding.role, pin);
        }

        info!("Claimed GPIO bindings for module '{}'", config.id);

        Ok(ClaimedModulePins {
            module_id: config.id.clone(),
            pins_by_role: claimed,
        })
    }

    pub fn release_module(&mut self, module_id: &str) {
        self.claims.retain(|_, claim| claim.module_id != module_id);
    }

    pub fn snapshot(&self, config: &ResourceConfig) -> ResourceConfigSnapshot {
        ResourceConfigSnapshot {
            board: self.board.snapshot(),
            module_instances: config
                .module_instances
                .iter()
                .map(|instance| ModuleInstanceSnapshot {
                    id: instance.id.clone(),
                    module_type: instance.module_type,
                    display_name: instance.display_name.clone(),
                    bindings: instance
                        .bindings
                        .iter()
                        .filter_map(|binding| {
                            binding.pin().ok().map(|pin| PinBindingSnapshot {
                                role: binding.role,
                                pin,
                                usage: binding.role.usage(),
                            })
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn validate_module(&self, module: &ModuleInstanceConfig) -> Result<(), AppError> {
        let required_roles = module.module_type.required_roles();
        let mut seen_roles = HashSet::new();

        for binding in &module.bindings {
            if !seen_roles.insert(binding.role) {
                return Err(AppError::Message(format!(
                    "Module '{}' defines role {:?} more than once",
                    module.id, binding.role
                )));
            }

            self.validate_binding(module, binding)?;
        }

        for role in required_roles {
            if !seen_roles.contains(&role) {
                return Err(AppError::Message(format!(
                    "Module '{}' is missing required role {:?}",
                    module.id, role
                )));
            }
        }

        Ok(())
    }

    fn validate_binding(
        &self,
        module: &ModuleInstanceConfig,
        binding: &PinBindingConfig,
    ) -> Result<(), AppError> {
        let pin = binding.pin()?;
        let usage = binding.role.usage();

        if !self.board.supports(pin, usage) {
            return Err(AppError::Message(format!(
                "GPIO{pin} does not support {:?} for module '{}' role {:?}",
                usage, module.id, binding.role
            )));
        }

        Ok(())
    }
}

impl ClaimedModulePins {
    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn pin_for(&self, role: ModuleRole) -> Option<u8> {
        self.pins_by_role.get(&role).copied()
    }
}
