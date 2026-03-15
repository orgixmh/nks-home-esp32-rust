use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use esp_idf_svc::hal::gpio::{AnyIOPin, Output, PinDriver};
use log::{error, info};

use crate::error::AppError;
use crate::runtime::{DegradedReason, OperationalStatus, RuntimeAction, RuntimeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorMode {
    Off,
    State(RuntimeState),
}

pub struct LedIndicator {
    command_tx: mpsc::Sender<IndicatorMode>,
}

impl LedIndicator {
    pub fn new_onboard() -> Result<Self, AppError> {
        let (command_tx, command_rx) = mpsc::channel();
        let mut led = create_led_driver()?;

        thread::Builder::new()
            .name("led-indicator".into())
            .stack_size(4096)
            .spawn(move || {
                if let Err(error) = run_indicator_loop(&mut led, command_rx) {
                    error!("LED indicator stopped: {error}");
                }
            })
            .map_err(|error| {
                AppError::Message(format!("Failed to spawn LED indicator: {error}"))
            })?;

        info!("LED indicator ready on GPIO2");

        Ok(Self { command_tx })
    }

    pub fn set_mode(&self, mode: IndicatorMode) {
        let _ = self.command_tx.send(mode);
    }
}

fn create_led_driver() -> Result<PinDriver<'static, AnyIOPin, Output>, AppError> {
    let pin = unsafe { AnyIOPin::new(2) };
    let mut led = PinDriver::output(pin)?;
    led.set_low()?;
    Ok(led)
}

fn run_indicator_loop(
    led: &mut PinDriver<'static, AnyIOPin, Output>,
    command_rx: mpsc::Receiver<IndicatorMode>,
) -> Result<(), AppError> {
    let mut current_mode = IndicatorMode::Off;
    led.set_low()?;

    loop {
        if let Ok(next_mode) = command_rx.try_recv() {
            current_mode = next_mode;
        }

        match current_mode {
            IndicatorMode::Off => {
                led.set_low()?;
                wait_for_next_mode(
                    command_rx.recv_timeout(Duration::from_millis(200)),
                    &mut current_mode,
                );
            }
            IndicatorMode::State(state) => match state.status {
                OperationalStatus::Provisioning => {
                    blink(
                        led,
                        &command_rx,
                        &mut current_mode,
                        Duration::from_millis(120),
                        Duration::from_millis(120),
                    )?;
                }
                OperationalStatus::Operational => {
                    if matches!(
                        state.action,
                        Some(RuntimeAction::WifiConnecting | RuntimeAction::MqttConnecting)
                    ) {
                        blink(
                            led,
                            &command_rx,
                            &mut current_mode,
                            Duration::from_millis(500),
                            Duration::from_millis(500),
                        )?;
                    } else {
                        led.set_high()?;
                        wait_for_next_mode(
                            command_rx.recv_timeout(Duration::from_millis(200)),
                            &mut current_mode,
                        );
                    }
                }
                OperationalStatus::Degraded => {
                    run_error_pattern(
                        led,
                        &command_rx,
                        &mut current_mode,
                        state.reason.unwrap_or(DegradedReason::Unknown),
                    )?;
                }
            },
        }
    }
}

fn blink(
    led: &mut PinDriver<'static, AnyIOPin, Output>,
    command_rx: &mpsc::Receiver<IndicatorMode>,
    current_mode: &mut IndicatorMode,
    on_time: Duration,
    off_time: Duration,
) -> Result<(), AppError> {
    led.set_high()?;
    if let Some(next_mode) = wait_interruptible(command_rx, on_time) {
        *current_mode = next_mode;
        return Ok(());
    }

    led.set_low()?;
    if let Some(next_mode) = wait_interruptible(command_rx, off_time) {
        *current_mode = next_mode;
    }

    Ok(())
}

fn run_error_pattern(
    led: &mut PinDriver<'static, AnyIOPin, Output>,
    command_rx: &mpsc::Receiver<IndicatorMode>,
    current_mode: &mut IndicatorMode,
    reason: DegradedReason,
) -> Result<(), AppError> {
    let flashes = match reason {
        DegradedReason::WifiDisconnected => 2,
        DegradedReason::MqttDisconnected => 4,
        DegradedReason::Unknown => 8,
    };

    led.set_low()?;
    if let Some(next_mode) = wait_interruptible(command_rx, Duration::from_secs(2)) {
        *current_mode = next_mode;
        return Ok(());
    }

    for _ in 0..flashes {
        led.set_high()?;
        if let Some(next_mode) = wait_interruptible(command_rx, Duration::from_millis(200)) {
            *current_mode = next_mode;
            return Ok(());
        }

        led.set_low()?;
        if let Some(next_mode) = wait_interruptible(command_rx, Duration::from_millis(200)) {
            *current_mode = next_mode;
            return Ok(());
        }
    }

    Ok(())
}

fn wait_interruptible(
    command_rx: &mpsc::Receiver<IndicatorMode>,
    duration: Duration,
) -> Option<IndicatorMode> {
    match command_rx.recv_timeout(duration) {
        Ok(next_mode) => Some(next_mode),
        Err(mpsc::RecvTimeoutError::Timeout) => None,
        Err(mpsc::RecvTimeoutError::Disconnected) => Some(IndicatorMode::Off),
    }
}

fn wait_for_next_mode(
    result: Result<IndicatorMode, mpsc::RecvTimeoutError>,
    current_mode: &mut IndicatorMode,
) {
    match result {
        Ok(next_mode) => *current_mode = next_mode,
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Err(mpsc::RecvTimeoutError::Disconnected) => *current_mode = IndicatorMode::Off,
    }
}
