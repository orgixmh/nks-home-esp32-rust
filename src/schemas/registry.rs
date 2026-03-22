use std::collections::HashMap;

use crate::error::AppError;

use super::builtins;
use super::types::{
    BindingRoleSchema, DeviceBindingMode, DeviceTypeSchema, DeviceTypeSchemaSnapshot,
    ModuleBindingSchemaSnapshot, ModuleSettingSchemaSnapshot, ModuleTypeSchema,
    ModuleTypeSchemaSnapshot,
};

pub struct SchemaRegistry {
    binding_roles: HashMap<&'static str, BindingRoleSchema>,
    module_types: HashMap<&'static str, ModuleTypeSchema>,
    device_types: HashMap<&'static str, DeviceTypeSchema>,
}

impl SchemaRegistry {
    pub fn built_in() -> Result<Self, AppError> {
        let registry = Self {
            binding_roles: HashMap::from([
                (builtins::ROLE_RELAY_OUTPUT.id, builtins::ROLE_RELAY_OUTPUT),
                (
                    builtins::ROLE_WALL_TRIGGER_INPUT.id,
                    builtins::ROLE_WALL_TRIGGER_INPUT,
                ),
                (builtins::ROLE_OUTPUT.id, builtins::ROLE_OUTPUT),
            ]),
            module_types: HashMap::from([
                (
                    builtins::MODULE_GPIO_SWITCH.id,
                    builtins::MODULE_GPIO_SWITCH,
                ),
                (
                    builtins::MODULE_GPIO_OUTPUT.id,
                    builtins::MODULE_GPIO_OUTPUT,
                ),
            ]),
            device_types: HashMap::from([(builtins::DEVICE_SWITCH.id, builtins::DEVICE_SWITCH)]),
        };

        registry.validate_internal()?;
        Ok(registry)
    }

    pub fn protocol_version(&self) -> &'static str {
        builtins::PROTOCOL_VERSION
    }

    pub fn schema_registry_version(&self) -> &'static str {
        builtins::SCHEMA_REGISTRY_VERSION
    }

    pub fn loaded_schema_packages(&self) -> Vec<String> {
        let mut values = builtins::LOADED_SCHEMA_PACKAGES
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        values.sort();
        values
    }

    pub fn supported_module_types(&self) -> Vec<String> {
        let mut values = self
            .module_types
            .keys()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        values.sort();
        values
    }

    pub fn lookup_binding_role(&self, id: &str) -> Option<&BindingRoleSchema> {
        self.binding_roles.get(id)
    }

    pub fn lookup_module_type(&self, id: &str) -> Option<&ModuleTypeSchema> {
        self.module_types.get(id)
    }

    pub fn lookup_device_type(&self, id: &str) -> Option<&DeviceTypeSchema> {
        self.device_types.get(id)
    }

    pub fn lookup_binding_role_required(&self, id: &str) -> Result<&BindingRoleSchema, AppError> {
        self.lookup_binding_role(id)
            .ok_or_else(|| AppError::Message(format!("Unknown binding role schema '{}'", id)))
    }

    pub fn lookup_module_type_required(&self, id: &str) -> Result<&ModuleTypeSchema, AppError> {
        self.lookup_module_type(id)
            .ok_or_else(|| AppError::Message(format!("Unknown module schema '{}'", id)))
    }

    pub fn lookup_device_type_required(&self, id: &str) -> Result<&DeviceTypeSchema, AppError> {
        self.lookup_device_type(id)
            .ok_or_else(|| AppError::Message(format!("Unknown device schema '{}'", id)))
    }

    pub fn module_type_snapshots(&self) -> Vec<ModuleTypeSchemaSnapshot> {
        let mut snapshots = self
            .module_types
            .values()
            .map(|schema| {
                let mut bindings = schema
                    .required_bindings
                    .iter()
                    .map(|role_id| ModuleBindingSchemaSnapshot {
                        role_id: (*role_id).to_string(),
                        usage: self
                            .binding_roles
                            .get(role_id)
                            .map(|role| role.resource_usage)
                            .unwrap_or(crate::config::types::ResourceUsage::Output),
                        required: true,
                        multiple: false,
                    })
                    .chain(schema.optional_bindings.iter().map(|role_id| {
                        ModuleBindingSchemaSnapshot {
                            role_id: (*role_id).to_string(),
                            usage: self
                                .binding_roles
                                .get(role_id)
                                .map(|role| role.resource_usage)
                                .unwrap_or(crate::config::types::ResourceUsage::Output),
                            required: false,
                            multiple: false,
                        }
                    }))
                    .collect::<Vec<_>>();
                bindings.sort_by(|left, right| left.role_id.cmp(&right.role_id));

                let mut settings = schema
                    .settings
                    .iter()
                    .map(|setting| ModuleSettingSchemaSnapshot {
                        key: setting.key.to_string(),
                        value_type: setting.value_type.to_string(),
                        required: setting.required,
                        ui_level: setting.ui_level.to_string(),
                    })
                    .collect::<Vec<_>>();
                settings.sort_by(|left, right| left.key.cmp(&right.key));

                ModuleTypeSchemaSnapshot {
                    id: schema.id.to_string(),
                    display_name: schema.display_name.to_string(),
                    device_binding_mode: schema.device_binding_mode.as_str().to_string(),
                    default_device_type_id: schema.default_device_type_id.map(str::to_string),
                    bindings,
                    settings,
                }
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.id.cmp(&right.id));
        snapshots
    }

    pub fn device_type_snapshots(&self) -> Vec<DeviceTypeSchemaSnapshot> {
        let mut snapshots = self
            .device_types
            .values()
            .map(|schema| DeviceTypeSchemaSnapshot {
                id: schema.id.to_string(),
                display_name: schema.display_name.to_string(),
                commands: schema
                    .commands
                    .iter()
                    .map(|command| super::types::DeviceCommandSchemaSnapshot {
                        name: command.name.to_string(),
                        automation_callable: command.automation_callable,
                    })
                    .collect(),
                state_fields: schema
                    .state_fields
                    .iter()
                    .map(|field| super::types::DeviceStateFieldSchemaSnapshot {
                        name: field.name.to_string(),
                        value_type: field.value_type.to_string(),
                        automation_readable: field.automation_readable,
                        operators: field
                            .operators
                            .iter()
                            .map(|value| (*value).to_string())
                            .collect(),
                    })
                    .collect(),
                capabilities: schema
                    .capabilities
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.id.cmp(&right.id));
        snapshots
    }

    fn validate_internal(&self) -> Result<(), AppError> {
        for schema in self.module_types.values() {
            let declared_binding_ids = schema
                .required_bindings
                .iter()
                .chain(schema.optional_bindings.iter())
                .copied()
                .collect::<Vec<_>>();

            for binding_id in schema
                .required_bindings
                .iter()
                .chain(schema.optional_bindings.iter())
            {
                if !self.binding_roles.contains_key(binding_id) {
                    return Err(AppError::Message(format!(
                        "Module schema '{}' references unknown binding role '{}'",
                        schema.id, binding_id
                    )));
                }
            }

            if !declared_binding_ids
                .iter()
                .any(|binding_id| *binding_id == schema.output_binding)
            {
                return Err(AppError::Message(format!(
                    "Module schema '{}' declares output binding '{}' outside its allowed bindings",
                    schema.id, schema.output_binding
                )));
            }

            if !self.binding_roles.contains_key(schema.output_binding) {
                return Err(AppError::Message(format!(
                    "Module schema '{}' references unknown output binding '{}'",
                    schema.id, schema.output_binding
                )));
            }

            if let Some(trigger_input_binding) = schema.trigger_input_binding {
                if !declared_binding_ids
                    .iter()
                    .any(|binding_id| *binding_id == trigger_input_binding)
                {
                    return Err(AppError::Message(format!(
                        "Module schema '{}' declares trigger binding '{}' outside its allowed bindings",
                        schema.id, trigger_input_binding
                    )));
                }

                if !self.binding_roles.contains_key(trigger_input_binding) {
                    return Err(AppError::Message(format!(
                        "Module schema '{}' references unknown trigger binding '{}'",
                        schema.id, trigger_input_binding
                    )));
                }
            }

            for device_type in schema.compatible_device_types {
                if !self.device_types.contains_key(device_type) {
                    return Err(AppError::Message(format!(
                        "Module schema '{}' references unknown device schema '{}'",
                        schema.id, device_type
                    )));
                }
            }

            if let Some(default_device_type_id) = schema.default_device_type_id {
                if !self.device_types.contains_key(default_device_type_id) {
                    return Err(AppError::Message(format!(
                        "Module schema '{}' references unknown default device schema '{}'",
                        schema.id, default_device_type_id
                    )));
                }

                if !schema
                    .compatible_device_types
                    .iter()
                    .any(|device_type| *device_type == default_device_type_id)
                {
                    return Err(AppError::Message(format!(
                        "Module schema '{}' default device schema '{}' is not listed as compatible",
                        schema.id, default_device_type_id
                    )));
                }
            }

            if matches!(schema.device_binding_mode, DeviceBindingMode::Multi)
                && schema.default_device_type_id.is_some()
            {
                return Err(AppError::Message(format!(
                    "Module schema '{}' cannot declare a default device schema for multi-device binding",
                    schema.id
                )));
            }
        }

        Ok(())
    }
}
