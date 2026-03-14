# AGENTS.md

## Project overview
This repository contains an ESP32 smart home firmware written in Rust on top of ESP-IDF.

## Architecture goals
- MQTT is the only inter-device transport.
- No direct controller-to-controller communication.
- OTA support will be added later.
- The design should remain modular and portable where practical.
- Security should be improved over the old firmware.
- Use normalized configuration structures and avoid ad hoc logic.

## Current state
- The project builds and flashes successfully.
- NVS-based config storage exists for Wi-Fi and MQTT settings.
- Boot logic already distinguishes between normal mode and provisioning mode.

## Coding rules
- Keep changes minimal and high confidence.
- Do not rewrite working modules unless necessary.
- Preserve the existing NVS config model unless explicitly asked to change it.
- Prefer clear module boundaries.
- Avoid unnecessary dependencies.
- Prefer compileable, incremental changes.
- Log important steps with clear messages.
- Do not add placeholders for future features unless they are directly needed.

## Near-term roadmap
1. Wi-Fi station connection using stored config from NVS.
2. MQTT connection using stored config from NVS.
3. Provisioning web interface for Wi-Fi and MQTT setup.

## Definition of done
For each task:
- Code must compile.
- Existing boot logic must remain functional.
- New functionality must be integrated into the current module layout.
