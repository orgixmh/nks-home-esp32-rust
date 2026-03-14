use core::convert::TryInto;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use log::info;

use crate::config::types::WifiConfig;
use crate::error::AppError;

pub fn connect_sta(cfg: &WifiConfig) -> Result<BlockingWifi<EspWifi<'static>>, AppError> {
    info!("Initializing Wi-Fi station mode");

    let peripherals = Peripherals::take()
        .ok_or_else(|| AppError::Message("Failed to take ESP peripherals".into()))?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let auth_method = if cfg.password.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    let wifi_configuration = Configuration::Client(ClientConfiguration {
        ssid: cfg
            .ssid
            .as_str()
            .try_into()
            .map_err(|_| AppError::Message("Wi-Fi SSID is too long".into()))?,
        password: cfg
            .password
            .as_str()
            .try_into()
            .map_err(|_| AppError::Message("Wi-Fi password is too long".into()))?,
        auth_method,
        ..Default::default()
    });

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;

    wifi.set_configuration(&wifi_configuration)?;
    wifi.start()?;
    info!("Wi-Fi started");

    wifi.connect()?;
    info!("Wi-Fi connected");

    wifi.wait_netif_up()?;
    info!("Wi-Fi netif up");

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    info!("Wi-Fi DHCP info: {ip_info:?}");

    Ok(wifi)
}
