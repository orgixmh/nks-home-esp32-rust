use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use log::info;

use crate::error::AppError;

const INDEX_HTML: &str = "<!doctype html><html><body><h1>Hello, world from esp32 and httpd!</h1></body></html>";

pub fn start_server() -> Result<EspHttpServer<'static>, AppError> {
    let mut server = EspHttpServer::new(&Configuration::default())?;

    server.fn_handler("/", Method::Get, |req| {
        req.into_ok_response()?
            .write_all(INDEX_HTML.as_bytes())
            .map(|_| ())
    })?;

    info!("HTTP server started");

    Ok(server)
}
