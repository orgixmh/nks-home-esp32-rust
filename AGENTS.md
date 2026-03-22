# AGENTS.md

## Project overview
This repository contains the h0m3 smart home system.

It currently includes:
- an ESP32 smart home firmware/backend written in Rust on top of ESP-IDF
- a browser-based frontend under `frontend/`

The firmware is the source of truth for runtime behavior, board capabilities, resource configuration, logical devices, and the MQTT contract.

## Core architecture principles
- MQTT is the only inter-device transport.
- No direct controller-to-controller communication.
- The system is schema-driven where practical.
- Runtime modules are the low-level execution/config layer.
- Logical devices are the public abstraction used by the frontend and automation.
- Frontend code must consume the public MQTT contract and must not invent a separate source of truth for device/module schemas.
- Avoid duplicated hand-maintained schemas across backend and frontend.
- Use normalized configuration structures and avoid ad hoc logic.
- Keep the design modular, portable where practical, and ready for future OTA/security improvements.

## MQTT contract principles
- `dev/...` topics are the public frontend/automation-facing contract.
- `mod/...` topics are internal/debug/runtime topics and should not be treated as the main frontend API.
- Public MQTT topic shapes should remain stable unless explicitly asked to change them.
- Prefer extending the schema-backed contract over adding one-off hardcoded topic behavior.

## Current backend state
- The firmware builds and flashes successfully.
- NVS-based config storage exists.
- Boot logic distinguishes between normal mode and provisioning mode.
- A built-in schema registry exists for core module/device definitions.
- Board/resources/devices/device-types are published over MQTT.
- Logical devices are part of the public abstraction layer.
- Current built-in core modules are modeled as simple single-device modules, but this should not be documented or implemented as a universal rule for all future module types.

## Frontend constraints
The frontend must follow these rules unless explicitly overridden:
- pure HTML/CSS/JavaScript only
- no Node.js
- no npm
- no bundler/build step
- must be able to run by opening `frontend/index.html` directly in a browser
- libraries are allowed only if they work directly in-browser without a build system
- frontend state/layout preferences should be stored in MQTT
- browser localStorage should only store lightweight browser-local preferences such as the selected default frontend config name
- theming must be supported from the beginning using CSS variables / theme tokens

## Coding rules
- Keep changes minimal, clear, and high confidence.
- Do not rewrite working modules unless necessary.
- Prefer compileable, incremental changes.
- Prefer clear module boundaries.
- Avoid unnecessary dependencies.
- Log important steps with clear messages.
- Do not add placeholders for future features unless they are directly needed.
- Do not introduce fake migration/versioning layers unless a real compatibility need exists.
- Do not hardcode frontend behavior that should come from the backend schema/MQTT contract.
- When extending the backend, prefer improving the schema registry / public MQTT contract instead of adding special-case logic in the frontend.
- Keep comments and documentation aligned with the actual current architecture, especially around module instances, logical devices, and public vs internal topics.
- Do not introduce a second hand-maintained schema definition inside the frontend when equivalent data can be published by the backend over MQTT.

## Near-term roadmap
1. Stabilize the public MQTT contract for frontend consumption.
2. Build the first browser-based frontend foundation in `frontend/`.
3. Support MQTT connection, device discovery, logical device tiles, theming, and MQTT-backed frontend configs.
4. Expand frontend support beyond the initial `core:switch` device type.
5. Continue evolving schema-driven configuration and future custom schema/module support carefully.

## Definition of done
For each task:
- Code must remain coherent with the current architecture.
- Backend changes must not silently break the public MQTT contract unless explicitly requested.
- Existing working runtime behavior must remain functional unless the task explicitly changes it.
- Frontend changes must work without Node.js or a build step.
- New functionality must fit the current module/device/schema structure rather than bypassing it.