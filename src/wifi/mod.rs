use core::convert::TryInto;
use core::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration,
    EspWifi, WifiDeviceId,
};
use log::info;
use serde::Serialize;

use crate::config::types::WifiConfig;
use crate::error::AppError;

pub fn connect_sta(
    modem: Modem,
    sys_loop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    cfg: &WifiConfig,
) -> Result<BlockingWifi<EspWifi<'static>>, AppError> {
    info!("Initializing Wi-Fi station mode");

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

    info!("Creating ESP Wi-Fi driver");
    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), Some(nvs))?, sys_loop)?;

    wifi.set_configuration(&wifi_configuration)?;
    info!("Wi-Fi configuration applied");
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

#[derive(Debug, Clone, Serialize)]
pub struct ScannedNetwork {
    pub ssid: String,
    pub channel: u8,
    pub signal_strength: i8,
    pub auth_required: bool,
}

pub struct ProvisioningWifi {
    wifi: BlockingWifi<EspWifi<'static>>,
    ap_config: AccessPointConfiguration,
    ap_ssid: String,
}

impl ProvisioningWifi {
    pub fn ap_ssid(&self) -> &str {
        &self.ap_ssid
    }

    pub fn scan_networks(&mut self) -> Result<Vec<ScannedNetwork>, AppError> {
        self.ensure_mixed_mode()?;

        let mut networks = self
            .wifi
            .scan()?
            .into_iter()
            .filter(|ap| !ap.ssid.is_empty())
            .map(|ap| ScannedNetwork {
                ssid: ap.ssid.to_string(),
                channel: ap.channel,
                signal_strength: ap.signal_strength,
                auth_required: ap
                    .auth_method
                    .is_some_and(|method| method != AuthMethod::None),
            })
            .collect::<Vec<_>>();

        networks.sort_by(|left, right| {
            right
                .signal_strength
                .cmp(&left.signal_strength)
                .then_with(|| left.ssid.cmp(&right.ssid))
        });
        networks.dedup_by(|left, right| left.ssid == right.ssid);

        Ok(networks)
    }

    pub fn test_sta_connection(&mut self, cfg: &WifiConfig) -> Result<String, AppError> {
        let client_cfg = client_configuration(cfg)?;

        self.wifi
            .set_configuration(&Configuration::Mixed(client_cfg, self.ap_config.clone()))?;
        info!("Testing Wi-Fi configuration for SSID '{}'", cfg.ssid);

        let result = (|| -> Result<String, AppError> {
            self.wifi.connect()?;
            self.wifi.ip_wait_while(
                || self.wifi.wifi().sta_netif().is_up().map(|is_up| !is_up),
                Some(Duration::from_secs(20)),
            )?;

            let ip_info = self.wifi.wifi().sta_netif().get_ip_info()?;
            Ok(ip_info.ip.to_string())
        })();

        let _ = self.wifi.disconnect();
        self.ensure_mixed_mode()?;
        let _ = self.wifi.ip_wait_while(
            || self.wifi.wifi().ap_netif().is_up().map(|is_up| !is_up),
            None,
        );

        result
    }

    fn ensure_mixed_mode(&mut self) -> Result<(), AppError> {
        self.wifi.set_configuration(&Configuration::Mixed(
            ClientConfiguration::default(),
            self.ap_config.clone(),
        ))?;

        Ok(())
    }
}

pub fn start_ap(
    modem: Modem,
    sys_loop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> Result<ProvisioningWifi, AppError> {
    info!("Initializing Wi-Fi access point mode");

    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), Some(nvs))?, sys_loop)?;
    let mac = wifi.wifi().get_mac(WifiDeviceId::Ap)?;
    let ssid = format!(
        "nks-home-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let ap_config = AccessPointConfiguration {
        ssid: ssid
            .as_str()
            .try_into()
            .map_err(|_| AppError::Message("Provisioning SSID is too long".into()))?,
        auth_method: AuthMethod::None,
        password: "".try_into().unwrap(),
        ..Default::default()
    };
    let wifi_configuration =
        Configuration::Mixed(ClientConfiguration::default(), ap_config.clone());

    wifi.set_configuration(&wifi_configuration)?;
    info!("Provisioning AP configuration applied");

    wifi.start()?;
    info!("Provisioning AP started: {ssid}");

    wifi.ip_wait_while(|| wifi.wifi().ap_netif().is_up().map(|is_up| !is_up), None)?;
    info!("Provisioning AP netif up");

    let ip_info = wifi.wifi().ap_netif().get_ip_info()?;
    info!("Provisioning AP IP info: {ip_info:?}");

    Ok(ProvisioningWifi {
        wifi,
        ap_config,
        ap_ssid: ssid,
    })
}

fn client_configuration(cfg: &WifiConfig) -> Result<ClientConfiguration, AppError> {
    let auth_method = if cfg.password.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    Ok(ClientConfiguration {
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
    })
}
