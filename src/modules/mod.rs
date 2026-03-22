mod events;
mod manager;
mod relay_gpio;
mod traits;

pub use manager::ModuleManager;

#[derive(Debug, Clone)]
pub struct ModuleCommand {
    pub module_id: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DriverBinaryState {
    On,
    Off,
}
