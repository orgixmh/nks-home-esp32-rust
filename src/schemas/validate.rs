use std::collections::HashSet;

use crate::config::types::{DeviceInstanceConfig, ModuleInstanceConfig, ResourceConfig};
use crate::error::AppError;

use super::registry::SchemaRegistry;

pub fn validate_module_instance(
    registry: &SchemaRegistry,
    module: &ModuleInstanceConfig,
) -> Result<(), AppError> {
    let schema = registry.lookup_module_type_required(&module.module_type_id)?;
    let mut seen_role_ids = HashSet::new();
    let allowed_role_ids = schema
        .required_bindings
        .iter()
        .chain(schema.optional_bindings.iter())
        .copied()
        .collect::<HashSet<_>>();

    for binding in &module.bindings {
        registry.lookup_binding_role_required(&binding.role_id)?;

        if !allowed_role_ids.contains(binding.role_id.as_str()) {
            return Err(AppError::Message(format!(
                "Module '{}' does not allow binding '{}'",
                module.id, binding.role_id
            )));
        }

        if !seen_role_ids.insert(binding.role_id.as_str()) {
            return Err(AppError::Message(format!(
                "Module '{}' defines schema role '{}' more than once",
                module.id, binding.role_id
            )));
        }
    }

    for required_binding in schema.required_bindings {
        if !seen_role_ids.contains(required_binding) {
            return Err(AppError::Message(format!(
                "Module '{}' is missing required binding '{}'",
                module.id, required_binding
            )));
        }
    }

    Ok(())
}

pub fn validate_device_instance(
    registry: &SchemaRegistry,
    config: &ResourceConfig,
    device: &DeviceInstanceConfig,
) -> Result<(), AppError> {
    let device_schema = registry.lookup_device_type_required(&device.device_type_id)?;
    let module = config
        .module_instances
        .iter()
        .find(|module| module.id == device.driver_module_id)
        .ok_or_else(|| {
            AppError::Message(format!(
                "Device '{}' references unknown driver module '{}'",
                device.id, device.driver_module_id
            ))
        })?;
    let module_schema = registry.lookup_module_type_required(&module.module_type_id)?;

    if !module_schema
        .compatible_device_types
        .iter()
        .any(|schema_id| *schema_id == device_schema.id)
    {
        return Err(AppError::Message(format!(
            "Module '{}' is not compatible with device '{}'",
            module.id, device.id
        )));
    }

    Ok(())
}
