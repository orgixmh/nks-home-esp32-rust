use std::time::Instant;

use crate::error::AppError;
use crate::mqtt::contract::MqttTopics;
use crate::mqtt::MqttManager;

use super::events::{ModuleEvent, ModuleExecution};
use super::DriverBinaryState;

pub trait Module {
    fn id(&self) -> &str;
    fn binary_state(&self) -> Option<DriverBinaryState> {
        None
    }
    fn publish_state(
        &mut self,
        mqtt: &mut MqttManager,
        topics: &MqttTopics,
    ) -> Result<(), AppError>;
    fn handle_command(&mut self, command: &str, now: Instant) -> Result<ModuleExecution, AppError>;
    fn handle_event(
        &mut self,
        event: &ModuleEvent,
        now: Instant,
    ) -> Result<ModuleExecution, AppError>;
    fn poll(&mut self, now: Instant) -> Result<ModuleExecution, AppError>;
}
