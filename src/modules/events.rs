use crate::modules::DriverBinaryState;

#[derive(Debug, Clone)]
pub(crate) struct ModuleEvent {
    pub source_module_id: String,
    pub kind: ModuleEventKind,
}

#[derive(Debug, Clone)]
pub(crate) enum ModuleEventKind {
    BinaryStateChanged { state: DriverBinaryState },
}

#[derive(Debug, Default)]
pub(crate) struct ModuleExecution {
    pub state_changed: bool,
    pub events: Vec<ModuleEvent>,
}
