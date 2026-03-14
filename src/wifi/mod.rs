use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, ClientConfiguration, Configuration, EspWifi};
use log::{error, info};

use crate::config::types::WifiConfig;
use crate::error::AppError;

pub fn connect_sta(cfg: &WifiConfig) -> Result<Box<EspWifi<'static>>, AppError> {
    info!("Initializing Wi-Fi station mode");

    let peripherals = Peripherals::take()?;

    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = Box::new(EspWifi::new(peripherals.modem, sysloop, Some(nvs))?);

    let auth_method = if cfg.password.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    let client_cfg = ClientConfiguration {
        ssid: cfg.ssid.as_str().try_into().map_err(|_| {
            AppError::Message("Wi-Fi SSID is too long for ESP-IDF configuration".to_string())
        })?,
        password: cfg.password.as_str().try_into().map_err(|_| {
            AppError::Message("Wi-Fi password is too long for ESP-IDF configuration".to_string())
        })?,
        auth_method,
        ..Default::default()
    };

    wifi.set_configuration(&Configuration::Client(client_cfg))?;

    info!("Connecting to Wi-Fi SSID '{}'", cfg.ssid);

    if let Err(e) = (|| -> Result<(), AppError> {
        wifi.start()?;
        wifi.connect()?;
        if !wifi.is_connected()? {
            return Err(AppError::Message(
                "Wi-Fi reported disconnected state after connect".to_string(),
            ));
        }
        Ok(())
    })() {
        error!("Wi-Fi connection failed for SSID '{}': {}", cfg.ssid, e);
        return Err(e);
    }

    info!("Wi-Fi connected successfully to SSID '{}'", cfg.ssid);

    Ok(wifi)
}
