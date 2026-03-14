use serde::Serialize;

use crate::config::types::ResourceUsage;

#[derive(Debug, Clone, Copy)]
pub struct BoardProfile {
    pub id: &'static str,
    pub name: &'static str,
    pub gpio_pins: &'static [BoardGpioPin],
}

#[derive(Debug, Clone, Copy)]
pub struct BoardGpioPin {
    pub number: u8,
    pub capabilities: GpioCapabilities,
    pub label: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct GpioCapabilities {
    pub input: bool,
    pub output: bool,
    pub pull_up: bool,
    pub pull_down: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardProfileSnapshot {
    pub id: String,
    pub name: String,
    pub gpio_pins: Vec<BoardGpioPinSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardGpioPinSnapshot {
    pub number: u8,
    pub label: String,
    pub input: bool,
    pub output: bool,
    pub pull_up: bool,
    pub pull_down: bool,
}

impl BoardProfile {
    pub fn esp32_devkit_v1() -> &'static Self {
        &ESP32_DEVKIT_V1
    }

    pub fn gpio_pin(&self, pin: u8) -> Option<&BoardGpioPin> {
        self.gpio_pins
            .iter()
            .find(|candidate| candidate.number == pin)
    }

    pub fn supports(&self, pin: u8, usage: ResourceUsage) -> bool {
        let Some(gpio_pin) = self.gpio_pin(pin) else {
            return false;
        };

        match usage {
            ResourceUsage::Input => gpio_pin.capabilities.input,
            ResourceUsage::Output => gpio_pin.capabilities.output,
        }
    }

    pub fn snapshot(&self) -> BoardProfileSnapshot {
        BoardProfileSnapshot {
            id: self.id.to_string(),
            name: self.name.to_string(),
            gpio_pins: self
                .gpio_pins
                .iter()
                .map(|pin| BoardGpioPinSnapshot {
                    number: pin.number,
                    label: pin.label.to_string(),
                    input: pin.capabilities.input,
                    output: pin.capabilities.output,
                    pull_up: pin.capabilities.pull_up,
                    pull_down: pin.capabilities.pull_down,
                })
                .collect(),
        }
    }
}

const IO_PIN: GpioCapabilities = GpioCapabilities {
    input: true,
    output: true,
    pull_up: true,
    pull_down: true,
};

const INPUT_ONLY_PIN: GpioCapabilities = GpioCapabilities {
    input: true,
    output: false,
    pull_up: false,
    pull_down: false,
};

const ESP32_DEVKIT_V1_PINS: [BoardGpioPin; 19] = [
    BoardGpioPin {
        number: 4,
        capabilities: IO_PIN,
        label: "GPIO4",
    },
    BoardGpioPin {
        number: 5,
        capabilities: IO_PIN,
        label: "GPIO5",
    },
    BoardGpioPin {
        number: 12,
        capabilities: IO_PIN,
        label: "GPIO12",
    },
    BoardGpioPin {
        number: 13,
        capabilities: IO_PIN,
        label: "GPIO13",
    },
    BoardGpioPin {
        number: 14,
        capabilities: IO_PIN,
        label: "GPIO14",
    },
    BoardGpioPin {
        number: 16,
        capabilities: IO_PIN,
        label: "GPIO16",
    },
    BoardGpioPin {
        number: 17,
        capabilities: IO_PIN,
        label: "GPIO17",
    },
    BoardGpioPin {
        number: 18,
        capabilities: IO_PIN,
        label: "GPIO18",
    },
    BoardGpioPin {
        number: 19,
        capabilities: IO_PIN,
        label: "GPIO19",
    },
    BoardGpioPin {
        number: 21,
        capabilities: IO_PIN,
        label: "GPIO21",
    },
    BoardGpioPin {
        number: 22,
        capabilities: IO_PIN,
        label: "GPIO22",
    },
    BoardGpioPin {
        number: 23,
        capabilities: IO_PIN,
        label: "GPIO23",
    },
    BoardGpioPin {
        number: 25,
        capabilities: IO_PIN,
        label: "GPIO25",
    },
    BoardGpioPin {
        number: 26,
        capabilities: IO_PIN,
        label: "GPIO26",
    },
    BoardGpioPin {
        number: 27,
        capabilities: IO_PIN,
        label: "GPIO27",
    },
    BoardGpioPin {
        number: 32,
        capabilities: IO_PIN,
        label: "GPIO32",
    },
    BoardGpioPin {
        number: 33,
        capabilities: IO_PIN,
        label: "GPIO33",
    },
    BoardGpioPin {
        number: 34,
        capabilities: INPUT_ONLY_PIN,
        label: "GPIO34",
    },
    BoardGpioPin {
        number: 35,
        capabilities: INPUT_ONLY_PIN,
        label: "GPIO35",
    },
];

const ESP32_DEVKIT_V1: BoardProfile = BoardProfile {
    id: "esp32-devkit-v1",
    name: "ESP32 DevKit V1",
    gpio_pins: &ESP32_DEVKIT_V1_PINS,
};
