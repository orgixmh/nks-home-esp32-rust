use serde::Serialize;

use crate::config::types::ResourceUsage;

#[derive(Debug, Clone, Copy)]
pub struct BindingRoleSchema {
    pub id: &'static str,
    pub resource_usage: ResourceUsage,
}

#[derive(Debug, Clone, Copy)]
pub enum DeviceBindingMode {
    Single,
    Multi,
}

#[derive(Debug, Clone, Copy)]
pub struct ModuleSettingSchema {
    pub key: &'static str,
    pub value_type: &'static str,
    pub required: bool,
    pub ui_level: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ModuleTypeSchema {
    pub id: &'static str,
    pub display_name: &'static str,
    pub runtime_driver: &'static str,
    pub required_bindings: &'static [&'static str],
    pub optional_bindings: &'static [&'static str],
    pub output_binding: &'static str,
    pub trigger_input_binding: Option<&'static str>,
    pub device_binding_mode: DeviceBindingMode,
    pub default_device_type_id: Option<&'static str>,
    pub compatible_device_types: &'static [&'static str],
    pub settings: &'static [ModuleSettingSchema],
    pub capabilities: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceCommandSchema {
    pub name: &'static str,
    pub automation_callable: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceStateFieldSchema {
    pub name: &'static str,
    pub value_type: &'static str,
    pub automation_readable: bool,
    pub operators: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceTypeSchema {
    pub id: &'static str,
    pub display_name: &'static str,
    pub commands: &'static [DeviceCommandSchema],
    pub state_fields: &'static [DeviceStateFieldSchema],
    pub capabilities: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ModuleBindingSchemaSnapshot {
    pub role_id: String,
    pub usage: ResourceUsage,
    pub required: bool,
    pub multiple: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleSettingSchemaSnapshot {
    pub key: String,
    pub value_type: String,
    pub required: bool,
    pub ui_level: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ModuleTypeSchemaSnapshot {
    pub id: String,
    pub display_name: String,
    pub device_binding_mode: String,
    pub default_device_type_id: Option<String>,
    pub bindings: Vec<ModuleBindingSchemaSnapshot>,
    pub settings: Vec<ModuleSettingSchemaSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceCommandSchemaSnapshot {
    pub name: String,
    pub automation_callable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceStateFieldSchemaSnapshot {
    pub name: String,
    pub value_type: String,
    pub automation_readable: bool,
    pub operators: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceTypeSchemaSnapshot {
    pub id: String,
    pub display_name: String,
    pub commands: Vec<DeviceCommandSchemaSnapshot>,
    pub state_fields: Vec<DeviceStateFieldSchemaSnapshot>,
    pub capabilities: Vec<String>,
}

impl DeviceBindingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multi => "multi",
        }
    }
}
