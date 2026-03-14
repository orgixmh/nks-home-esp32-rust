mod app;
mod board;
mod config;
mod error;
mod gpio;
mod http;
mod modules;
mod mqtt;
mod runtime;
mod storage;
mod wifi;

use crate::error::AppError;
use crate::runtime::AppController;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

fn main() -> Result<(), AppError> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let nvs = EspDefaultNvsPartition::take()?;
    AppController::new(nvs).run()
}
