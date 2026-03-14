use crate::error::AppError;
use crate::storage::nvs::ConfigStore;

pub enum BootMode {
    Normal,
    Provisioning,
}

pub fn detect_boot_mode() -> Result<BootMode, AppError> {
    let store = ConfigStore::new()?;
    let cfg = store.load()?;

    match cfg {
        Some(cfg) if cfg.is_complete() => Ok(BootMode::Normal),
        _ => Ok(BootMode::Provisioning),
    }
}
