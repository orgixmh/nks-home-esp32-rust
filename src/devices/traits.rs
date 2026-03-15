use crate::error::AppError;
use crate::modules::ModuleManager;
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;

#[derive(Debug, Clone)]
pub struct DeviceCommand {
    pub device_id: String,
    pub payload: Vec<u8>,
}

pub trait DeviceController {
    fn id(&self) -> &str;
    fn driver_module_id(&self) -> &str;
    fn publish_state(
        &self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
        modules: &ModuleManager,
    ) -> Result<(), AppError>;
    fn handle_command(
        &self,
        payload: &[u8],
        modules: &mut ModuleManager,
    ) -> Result<Vec<String>, AppError>;
}
