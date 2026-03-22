use crate::config::types::ResourceUsage;

use super::types::{
    BindingRoleSchema, DeviceBindingMode, DeviceCommandSchema, DeviceStateFieldSchema,
    DeviceTypeSchema, ModuleSettingSchema, ModuleTypeSchema,
};

pub const PROTOCOL_VERSION: &str = "1";
pub const SCHEMA_REGISTRY_VERSION: &str = "builtin-1";
pub const LOADED_SCHEMA_PACKAGES: &[&str] = &["core"];

pub const ROLE_RELAY_OUTPUT: BindingRoleSchema = BindingRoleSchema {
    id: "core:relay_output",
    resource_usage: ResourceUsage::Output,
};
pub const ROLE_WALL_TRIGGER_INPUT: BindingRoleSchema = BindingRoleSchema {
    id: "core:wall_trigger_input",
    resource_usage: ResourceUsage::Input,
};
pub const ROLE_OUTPUT: BindingRoleSchema = BindingRoleSchema {
    id: "core:output",
    resource_usage: ResourceUsage::Output,
};

const MODULE_SWITCH_SETTINGS: &[ModuleSettingSchema] = &[
    ModuleSettingSchema {
        key: "auto_off_ms",
        value_type: "integer",
        required: false,
        ui_level: "basic",
    },
    ModuleSettingSchema {
        key: "external_on_triggers",
        value_type: "array",
        required: false,
        ui_level: "advanced",
    },
];

pub const DEVICE_SWITCH: DeviceTypeSchema = DeviceTypeSchema {
    id: "core:switch",
    display_name: "Switch",
    commands: &[
        DeviceCommandSchema {
            name: "ON",
            automation_callable: true,
        },
        DeviceCommandSchema {
            name: "OFF",
            automation_callable: true,
        },
        DeviceCommandSchema {
            name: "TOGGLE",
            automation_callable: true,
        },
    ],
    state_fields: &[DeviceStateFieldSchema {
        name: "state",
        value_type: "string",
        automation_readable: true,
        operators: &["=="],
    }],
    capabilities: &["binary_output", "retained_state"],
};

pub const MODULE_GPIO_SWITCH: ModuleTypeSchema = ModuleTypeSchema {
    id: "core:gpio_switch",
    display_name: "GPIO Switch",
    runtime_driver: "relay_gpio",
    required_bindings: &[ROLE_RELAY_OUTPUT.id],
    optional_bindings: &[ROLE_WALL_TRIGGER_INPUT.id],
    output_binding: ROLE_RELAY_OUTPUT.id,
    trigger_input_binding: Some(ROLE_WALL_TRIGGER_INPUT.id),
    device_binding_mode: DeviceBindingMode::Single,
    default_device_type_id: Some(DEVICE_SWITCH.id),
    compatible_device_types: &[DEVICE_SWITCH.id],
    settings: MODULE_SWITCH_SETTINGS,
    capabilities: &["binary_output", "auto_off", "gpio_toggle_trigger"],
};

pub const MODULE_GPIO_OUTPUT: ModuleTypeSchema = ModuleTypeSchema {
    id: "core:gpio_output",
    display_name: "GPIO Output",
    runtime_driver: "relay_gpio",
    required_bindings: &[ROLE_OUTPUT.id],
    optional_bindings: &[],
    output_binding: ROLE_OUTPUT.id,
    trigger_input_binding: None,
    device_binding_mode: DeviceBindingMode::Single,
    default_device_type_id: Some(DEVICE_SWITCH.id),
    compatible_device_types: &[DEVICE_SWITCH.id],
    settings: MODULE_SWITCH_SETTINGS,
    capabilities: &["binary_output"],
};
