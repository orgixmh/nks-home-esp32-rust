use std::sync::{Arc, Mutex};
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

const INDICATOR_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub struct LedIndicator {
    mode: Arc<Mutex<IndicatorMode>>,
}

impl LedIndicator {
    pub fn new_onboard() -> Result<Self, AppError> {
        let mode = Arc::new(Mutex::new(IndicatorMode::Off));
        let mode_for_thread = mode.clone();
        let mut led = create_led_driver()?;

        thread::Builder::new()
            .name("led-indicator".into())
            .stack_size(4096)
            .spawn(move || {
                if let Err(error) = run_indicator_loop(&mut led, mode_for_thread) {
                    error!("LED indicator stopped: {error}");
                }
            })
            .map_err(|error| {
                AppError::Message(format!("Failed to spawn LED indicator: {error}"))
            })?;

        info!("LED indicator ready on GPIO2");

        Ok(Self { mode })
    }

    pub fn set_mode(&self, mode: IndicatorMode) {
        if let Ok(mut current) = self.mode.lock() {
            *current = mode;
        }
    }

    pub fn tick(&mut self) {}
}

fn create_led_driver() -> Result<PinDriver<'static, AnyIOPin, Output>, AppError> {
    let pin = unsafe { AnyIOPin::new(2) };
    let mut led = PinDriver::output(pin)?;
    led.set_low()?;
    Ok(led)
}

fn run_indicator_loop(
    led: &mut PinDriver<'static, AnyIOPin, Output>,
    mode: Arc<Mutex<IndicatorMode>>,
) -> Result<(), AppError> {
    let mut current_mode = IndicatorMode::Off;
    led.set_low()?;

    loop {
        current_mode = read_mode(&mode, current_mode);

        match current_mode {
            IndicatorMode::Off => {
                led.set_low()?;
                current_mode = wait_for_mode_change(&mode, current_mode, Duration::from_millis(200));
            }
            IndicatorMode::State(state) => match state.status {
                OperationalStatus::Provisioning => {
                    blink(
                        led,
                        &mode,
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
                            &mode,
                            &mut current_mode,
                            Duration::from_millis(500),
                            Duration::from_millis(500),
                        )?;
                    } else {
                        led.set_high()?;
                        current_mode =
                            wait_for_mode_change(&mode, current_mode, Duration::from_millis(200));
                    }
                }
                OperationalStatus::Degraded => {
                    run_error_pattern(
                        led,
                        &mode,
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
    mode: &Arc<Mutex<IndicatorMode>>,
    current_mode: &mut IndicatorMode,
    on_time: Duration,
    off_time: Duration,
) -> Result<(), AppError> {
    led.set_high()?;
    if let Some(next_mode) = wait_interruptible(mode, *current_mode, on_time) {
        *current_mode = next_mode;
        return Ok(());
    }

    led.set_low()?;
    if let Some(next_mode) = wait_interruptible(mode, *current_mode, off_time) {
        *current_mode = next_mode;
    }

    Ok(())
}

fn run_error_pattern(
    led: &mut PinDriver<'static, AnyIOPin, Output>,
    mode: &Arc<Mutex<IndicatorMode>>,
    current_mode: &mut IndicatorMode,
    reason: DegradedReason,
) -> Result<(), AppError> {
    let flashes = match reason {
        DegradedReason::WifiDisconnected => 2,
        DegradedReason::MqttDisconnected => 4,
        DegradedReason::Unknown => 8,
    };

    led.set_low()?;
    if let Some(next_mode) = wait_interruptible(mode, *current_mode, Duration::from_secs(2)) {
        *current_mode = next_mode;
        return Ok(());
    }

    for _ in 0..flashes {
        led.set_high()?;
        if let Some(next_mode) = wait_interruptible(mode, *current_mode, Duration::from_millis(200))
        {
            *current_mode = next_mode;
            return Ok(());
        }

        led.set_low()?;
        if let Some(next_mode) = wait_interruptible(mode, *current_mode, Duration::from_millis(200))
        {
            *current_mode = next_mode;
            return Ok(());
        }
    }

    Ok(())
}

fn wait_interruptible(
    mode: &Arc<Mutex<IndicatorMode>>,
    current_mode: IndicatorMode,
    duration: Duration,
) -> Option<IndicatorMode> {
    let mut remaining = duration;

    loop {
        let next_mode = read_mode(mode, current_mode);
        if next_mode != current_mode {
            return Some(next_mode);
        }

        if remaining.is_zero() {
            return None;
        }

        let sleep_for = remaining.min(INDICATOR_POLL_INTERVAL);
        thread::sleep(sleep_for);
        remaining = remaining.saturating_sub(sleep_for);
    }
}

fn wait_for_mode_change(
    mode: &Arc<Mutex<IndicatorMode>>,
    current_mode: IndicatorMode,
    duration: Duration,
) -> IndicatorMode {
    wait_interruptible(mode, current_mode, duration).unwrap_or(current_mode)
}

fn read_mode(mode: &Arc<Mutex<IndicatorMode>>, fallback: IndicatorMode) -> IndicatorMode {
    mode.lock().map(|value| *value).unwrap_or(fallback)
}
