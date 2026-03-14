Project rules for nks-home-esp32-rust:

- This project targets ESP32 using Rust + esp-idf.
- Never assume the build is green unless you ran the local check action successfully.
- Before any commit, run: ./scripts/codex-check.sh
- If setup/check fails because ESP-IDF tools are missing, stop and report the environment issue instead of committing.
- Do not replace ESP32-specific code with desktop-only stubs just to satisfy compilation.
- Prefer minimal, incremental changes.
- For Wi-Fi, MQTT, and provisioning work, keep changes separated into small commits after a successful local check.
