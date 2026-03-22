Project rules for nks-home-esp32-rust:

General:
- This repository contains both:
  - ESP32 firmware/backend code written in Rust + esp-idf
  - a browser frontend under `frontend/`
- Prefer minimal, incremental changes.
- Do not assume anything is correct unless you validated the relevant part of the repo.
- Do not introduce unnecessary dependencies or large rewrites.

Firmware / backend rules:
- The firmware targets ESP32 using Rust + esp-idf.
- Never assume the firmware build is green unless you ran the local check action successfully.
- Before any firmware/backend commit, run: `./scripts/codex-check.sh`
- If setup/check fails because ESP-IDF tools are missing, stop and report the environment issue instead of committing.
- Do not replace ESP32-specific code with desktop-only stubs just to satisfy compilation.
- Keep firmware changes small and high confidence.
- For Wi-Fi, MQTT, provisioning, schema registry, and backend MQTT contract work, prefer separated small commits after a successful local check.
- When changing MQTT topics, payload shapes, or schema-facing backend contracts, update related frontend-facing documentation in the same task unless explicitly asked not to.

Frontend rules:
- The frontend must remain pure HTML/CSS/JavaScript.
- No Node.js, npm, bundlers, or build step.
- Frontend code must be able to run by opening `frontend/index.html` directly in a browser.
- Use browser-compatible libraries only when they work directly without a build system.
- Do not introduce a second hand-maintained source of truth for schemas in the frontend when equivalent data can come from the backend MQTT contract.
- Use CSS variables / theme tokens for theming from the beginning.

Validation expectations:
- If a change touches firmware/backend code, run `./scripts/codex-check.sh`.
- If a change touches only `frontend/`, do not run firmware-only checks unless the task also changes backend code.
- If a change spans both backend and frontend, validate both sides as appropriate and do not claim success based on only one side.

Architecture expectations:
- `dev/...` topics are the public frontend/automation-facing MQTT contract.
- `mod/...` topics are internal/debug/runtime topics.
- Prefer improving the schema-driven backend contract instead of hardcoding special cases in the frontend.
- Keep comments and documentation aligned with the actual current architecture.