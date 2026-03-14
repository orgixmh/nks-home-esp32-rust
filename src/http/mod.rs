use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use esp_idf_svc::http::server::Connection;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use log::info;
use serde::{Deserialize, Serialize};

use crate::config::types::MqttConfig;
use crate::config::types::WifiConfig;
use crate::error::AppError;
use crate::storage::nvs::ConfigStore;
use crate::wifi::ScannedNetwork;

const INDEX_HTML: &str = include_str!("assets/index.html");
const PROVISIONING_HTML: &str = include_str!("assets/provisioning.html");
const PROVISIONING_CSS: &str = include_str!("assets/provisioning.css");
const PROVISIONING_JS: &str = include_str!("assets/provisioning.js");

#[derive(Clone)]
pub struct ProvisioningController {
    state: Arc<Mutex<ProvisioningState>>,
}

struct ProvisioningState {
    store: ConfigStore,
    ap_ssid: String,
    cached_networks: Vec<ScannedNetwork>,
    pending_wifi_test: Option<WifiConfig>,
    last_successful_wifi: Option<WifiConfig>,
    wifi_test_status: WifiTestStatus,
    pending_mqtt_test: Option<MqttConfig>,
    last_successful_mqtt: Option<MqttConfig>,
    mqtt_test_status: WifiTestStatus,
}

#[derive(Clone, Serialize)]
pub struct WifiTestStatus {
    pub state: &'static str,
    pub message: String,
    pub ip: Option<String>,
    pub error_code: Option<i32>,
}

#[derive(Deserialize)]
struct WifiCredentialsPayload {
    ssid: String,
    password: String,
}

#[derive(Deserialize)]
struct MqttPayload {
    protocol: String,
    broker: String,
    username: String,
    password: String,
}

#[derive(Serialize)]
struct NetworksResponse {
    networks: Vec<ScannedNetwork>,
    cached: bool,
}

#[derive(Serialize)]
struct ProvisioningStatusResponse {
    ap_ssid: String,
}

#[derive(Serialize)]
struct WifiActionResponse {
    ok: bool,
    message: String,
}

impl ProvisioningController {
    pub fn new(store: ConfigStore, ap_ssid: String, cached_networks: Vec<ScannedNetwork>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProvisioningState {
                store,
                ap_ssid,
                cached_networks,
                pending_wifi_test: None,
                last_successful_wifi: None,
                wifi_test_status: WifiTestStatus {
                    state: "idle",
                    message: "Ready to connect H0m3.".into(),
                    ip: None,
                    error_code: None,
                },
                pending_mqtt_test: None,
                last_successful_mqtt: None,
                mqtt_test_status: WifiTestStatus {
                    state: "idle",
                    message: "Ready to connect H0m3 to your broker.".into(),
                    ip: None,
                    error_code: None,
                },
            })),
        }
    }

    pub fn schedule_wifi_test(&self, wifi: WifiConfig) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.pending_wifi_test = Some(wifi);
        state.last_successful_wifi = None;
        state.wifi_test_status = WifiTestStatus {
            state: "scheduled",
            message: "Connecting H0m3 to your Wi-Fi.".into(),
            ip: None,
            error_code: None,
        };

        Ok(())
    }

    pub fn take_pending_wifi_test(&self) -> Result<Option<WifiConfig>, AppError> {
        Ok(self.lock()?.pending_wifi_test.take())
    }

    pub fn mark_wifi_test_running(&self, ssid: &str) -> Result<(), AppError> {
        self.lock()?.wifi_test_status = WifiTestStatus {
            state: "testing",
            message: format!("Checking connection to '{ssid}'"),
            ip: None,
            error_code: None,
        };

        Ok(())
    }

    pub fn mark_wifi_test_success(&self, wifi: WifiConfig, ip: String) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.last_successful_wifi = Some(wifi);
        state.wifi_test_status = WifiTestStatus {
            state: "success",
            message: "H0m3 connected successfully.".into(),
            ip: Some(ip),
            error_code: None,
        };

        Ok(())
    }

    pub fn mark_wifi_test_error(&self, error: &AppError) -> Result<(), AppError> {
        self.lock()?.wifi_test_status = WifiTestStatus {
            state: "error",
            message: error.to_string(),
            ip: None,
            error_code: extract_esp_code(error),
        };

        Ok(())
    }

    pub fn wifi_test_status(&self) -> Result<WifiTestStatus, AppError> {
        Ok(self.lock()?.wifi_test_status.clone())
    }

    pub fn ap_ssid(&self) -> Result<String, AppError> {
        Ok(self.lock()?.ap_ssid.clone())
    }

    pub fn cached_networks(&self) -> Result<Vec<ScannedNetwork>, AppError> {
        Ok(self.lock()?.cached_networks.clone())
    }

    pub fn save_tested_wifi(&self, wifi: &WifiConfig) -> Result<(), AppError> {
        let mut state = self.lock()?;
        let Some(last_successful_wifi) = &state.last_successful_wifi else {
            return Err(AppError::Message(
                "Please confirm your Wi-Fi connection before continuing.".into(),
            ));
        };

        if last_successful_wifi.ssid != wifi.ssid || last_successful_wifi.password != wifi.password
        {
            return Err(AppError::Message(
                "Please use the same Wi-Fi details you just confirmed.".into(),
            ));
        }

        state.store.save_wifi(wifi)?;
        state.wifi_test_status = WifiTestStatus {
            state: "saved",
            message: "Wi-Fi saved. You're ready for the next step.".into(),
            ip: None,
            error_code: None,
        };

        Ok(())
    }

    pub fn wifi_for_mqtt_test(&self) -> Result<WifiConfig, AppError> {
        let state = self.lock()?;
        state.last_successful_wifi.clone().ok_or_else(|| {
            AppError::Message("Please finish the Wi-Fi step before broker setup.".into())
        })
    }

    pub fn schedule_mqtt_test(&self, payload: MqttPayload) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.pending_mqtt_test = Some(build_mqtt_config(&state.ap_ssid, payload));
        state.last_successful_mqtt = None;
        state.mqtt_test_status = WifiTestStatus {
            state: "scheduled",
            message: "Connecting H0m3 to your broker.".into(),
            ip: None,
            error_code: None,
        };

        Ok(())
    }

    pub fn take_pending_mqtt_test(&self) -> Result<Option<MqttConfig>, AppError> {
        Ok(self.lock()?.pending_mqtt_test.take())
    }

    pub fn mark_mqtt_test_running(&self, host: &str) -> Result<(), AppError> {
        self.lock()?.mqtt_test_status = WifiTestStatus {
            state: "testing",
            message: format!("Checking broker connection to '{host}'"),
            ip: None,
            error_code: None,
        };

        Ok(())
    }

    pub fn mark_mqtt_test_success(&self, mqtt: MqttConfig) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.last_successful_mqtt = Some(mqtt);
        state.mqtt_test_status = WifiTestStatus {
            state: "success",
            message: "H0m3 connected to your broker successfully.".into(),
            ip: None,
            error_code: None,
        };

        Ok(())
    }

    pub fn mark_mqtt_test_error(&self, error: &AppError) -> Result<(), AppError> {
        self.lock()?.mqtt_test_status = WifiTestStatus {
            state: "error",
            message: error.to_string(),
            ip: None,
            error_code: extract_esp_code(error),
        };

        Ok(())
    }

    pub fn mqtt_test_status(&self) -> Result<WifiTestStatus, AppError> {
        Ok(self.lock()?.mqtt_test_status.clone())
    }

    pub fn save_tested_mqtt(&self, payload: MqttPayload) -> Result<String, AppError> {
        let mut state = self.lock()?;
        let mqtt = build_mqtt_config(&state.ap_ssid, payload);
        let Some(last_successful_mqtt) = &state.last_successful_mqtt else {
            return Err(AppError::Message(
                "Please confirm your broker connection before finishing.".into(),
            ));
        };

        if !same_mqtt_config(last_successful_mqtt, &mqtt) {
            return Err(AppError::Message(
                "Please use the same broker details you just confirmed.".into(),
            ));
        }

        state.store.save_mqtt(&mqtt)?;
        state.mqtt_test_status = WifiTestStatus {
            state: "saved",
            message: "H0m3 is ready and will reboot to apply your settings.".into(),
            ip: None,
            error_code: None,
        };

        Ok("H0m3 is ready and will reboot to apply your settings.".into())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ProvisioningState>, AppError> {
        self.state
            .lock()
            .map_err(|_| AppError::Message("Provisioning backend lock poisoned".into()))
    }
}

pub fn start_server(store: ConfigStore) -> Result<EspHttpServer<'static>, AppError> {
    let mut server = EspHttpServer::new(&server_config())?;
    let store = Arc::new(Mutex::new(store));

    server.fn_handler("/", Method::Get, |req| {
        write_response(
            req,
            200,
            "OK",
            "text/html; charset=utf-8",
            INDEX_HTML.as_bytes(),
        )
    })?;

    {
        let store = store.clone();
        server.fn_handler::<AppError, _>("/api/reset-configuration", Method::Post, move |req| {
            store
                .lock()
                .map_err(|_| AppError::Message("Config store lock poisoned".into()))?
                .clear_all()?;

            write_json(
                req,
                200,
                "OK",
                &WifiActionResponse {
                    ok: true,
                    message: "Configuration cleared. Rebooting device.".into(),
                },
            )?;

            thread::spawn(|| {
                thread::sleep(Duration::from_millis(500));
                unsafe {
                    esp_idf_svc::sys::esp_restart();
                }
            });

            Ok(())
        })?;
    }

    info!("HTTP server started");

    Ok(server)
}

pub fn start_captive_portal_server(
    controller: ProvisioningController,
) -> Result<EspHttpServer<'static>, AppError> {
    let mut server = EspHttpServer::new(&server_config())?;

    server.fn_handler("/", Method::Get, |req| {
        write_response(
            req,
            200,
            "OK",
            "text/html; charset=utf-8",
            PROVISIONING_HTML.as_bytes(),
        )
    })?;

    server.fn_handler("/assets/provisioning.css", Method::Get, |req| {
        write_response(
            req,
            200,
            "OK",
            "text/css; charset=utf-8",
            PROVISIONING_CSS.as_bytes(),
        )
    })?;

    server.fn_handler("/assets/provisioning.js", Method::Get, |req| {
        write_response(
            req,
            200,
            "OK",
            "application/javascript; charset=utf-8",
            PROVISIONING_JS.as_bytes(),
        )
    })?;

    {
        let controller = controller.clone();
        server.fn_handler("/api/provisioning/status", Method::Get, move |req| {
            let payload = ProvisioningStatusResponse {
                ap_ssid: controller.ap_ssid()?,
            };

            write_json(req, 200, "OK", &payload)
        })?;
    }

    {
        let controller = controller.clone();
        server.fn_handler("/api/networks", Method::Get, move |req| {
            let payload = NetworksResponse {
                networks: controller.cached_networks()?,
                cached: true,
            };

            write_json(req, 200, "OK", &payload)
        })?;
    }

    {
        let controller = controller.clone();
        server.fn_handler("/api/test-wifi", Method::Post, move |mut req| {
            let payload: WifiCredentialsPayload = read_json_body(&mut req)?;
            let wifi_cfg = WifiConfig {
                ssid: payload.ssid,
                password: payload.password,
            };

            controller.schedule_wifi_test(wifi_cfg)?;

            write_json(
                req,
                200,
                "OK",
                &WifiActionResponse {
                    ok: true,
                    message: "Wi-Fi test started. Polling for result.".into(),
                },
            )
        })?;
    }

    {
        let controller = controller.clone();
        server.fn_handler("/api/wifi-status", Method::Get, move |req| {
            let payload = controller.wifi_test_status()?;
            write_json(req, 200, "OK", &payload)
        })?;
    }

    {
        let controller = controller.clone();
        server.fn_handler("/api/save-wifi", Method::Post, move |mut req| {
            let payload: WifiCredentialsPayload = read_json_body(&mut req)?;
            let wifi_cfg = WifiConfig {
                ssid: payload.ssid,
                password: payload.password,
            };

            controller.save_tested_wifi(&wifi_cfg)?;

            write_json(
                req,
                200,
                "OK",
                &WifiActionResponse {
                    ok: true,
                    message: "Wi-Fi configuration saved. MQTT setup is next.".into(),
                },
            )
        })?;
    }

    {
        let controller = controller.clone();
        server.fn_handler("/api/test-mqtt", Method::Post, move |mut req| {
            let payload: MqttPayload = read_json_body(&mut req)?;

            controller.schedule_mqtt_test(payload)?;

            write_json(
                req,
                200,
                "OK",
                &WifiActionResponse {
                    ok: true,
                    message: "Broker connection check started.".into(),
                },
            )
        })?;
    }

    {
        let controller = controller.clone();
        server.fn_handler("/api/mqtt-status", Method::Get, move |req| {
            let payload = controller.mqtt_test_status()?;
            write_json(req, 200, "OK", &payload)
        })?;
    }

    {
        let controller = controller.clone();
        server.fn_handler("/api/save-mqtt", Method::Post, move |mut req| {
            let payload: MqttPayload = read_json_body(&mut req)?;
            let message = controller.save_tested_mqtt(payload)?;

            thread::spawn(|| {
                thread::sleep(Duration::from_secs(3));
                unsafe {
                    esp_idf_svc::sys::esp_restart();
                }
            });

            write_json(req, 200, "OK", &WifiActionResponse { ok: true, message })
        })?;
    }

    server.fn_handler("/*", Method::Get, |req| {
        write_response(
            req,
            200,
            "OK",
            "text/html; charset=utf-8",
            PROVISIONING_HTML.as_bytes(),
        )
    })?;

    info!("Provisioning HTTP server started");

    Ok(server)
}

fn server_config() -> Configuration {
    Configuration {
        stack_size: 10240,
        uri_match_wildcard: true,
        max_uri_handlers: 12,
        ..Default::default()
    }
}

fn read_json_body<T: for<'de> Deserialize<'de>, C>(
    req: &mut esp_idf_svc::http::server::Request<C>,
) -> Result<T, AppError>
where
    C: Connection<Error = esp_idf_svc::io::EspIOError>,
{
    let mut body = Vec::new();
    let mut buf = [0_u8; 512];

    loop {
        let read = req.read(&mut buf)?;

        if read == 0 {
            break;
        }

        body.extend_from_slice(&buf[..read]);

        if body.len() > 4096 {
            return Err(AppError::Message("Request body too large".into()));
        }
    }

    if body.is_empty() {
        return Err(AppError::Message("Request body is empty".into()));
    }

    Ok(serde_json::from_slice(&body)?)
}

fn write_json<T: Serialize, C>(
    req: esp_idf_svc::http::server::Request<C>,
    status: u16,
    status_message: &str,
    payload: &T,
) -> Result<(), AppError>
where
    C: Connection<Error = esp_idf_svc::io::EspIOError>,
{
    let body = serde_json::to_vec(payload)?;
    write_response(
        req,
        status,
        status_message,
        "application/json; charset=utf-8",
        &body,
    )
}

fn write_response<C>(
    req: esp_idf_svc::http::server::Request<C>,
    status: u16,
    status_message: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), AppError>
where
    C: Connection<Error = esp_idf_svc::io::EspIOError>,
{
    req.into_response(
        status,
        Some(status_message),
        &[("Content-Type", content_type)],
    )?
    .write_all(body)?;

    Ok(())
}

fn extract_esp_code(error: &AppError) -> Option<i32> {
    match error {
        AppError::Esp(e) => Some(e.code()),
        AppError::EspIo(e) => Some(e.0.code()),
        _ => None,
    }
}

fn build_mqtt_config(ap_ssid: &str, payload: MqttPayload) -> MqttConfig {
    let device_id = ap_ssid.to_lowercase();
    MqttConfig {
        host: payload.broker,
        port: if payload.protocol.eq_ignore_ascii_case("ssl") {
            8883
        } else {
            1883
        },
        username: payload.username,
        password: payload.password,
        client_id: device_id.clone(),
        base_topic: format!("nks/home/{device_id}"),
    }
}

fn same_mqtt_config(left: &MqttConfig, right: &MqttConfig) -> bool {
    left.host == right.host
        && left.port == right.port
        && left.username == right.username
        && left.password == right.password
        && left.client_id == right.client_id
        && left.base_topic == right.base_topic
}
