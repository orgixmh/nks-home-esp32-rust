const state = {
  currentStep: 0,
  hasLoadedWifiStep: false,
};

const stepViews = Array.from(document.querySelectorAll("[data-step]"));
const stepIndicators = Array.from(document.querySelectorAll("[data-step-indicator]"));
const loadingOverlay = document.getElementById("loading-overlay");
const loadingMessage = document.getElementById("loading-message");
const wifiForm = document.getElementById("wifi-form");
const mqttForm = document.getElementById("mqtt-form");
const wifiNextButton = document.getElementById("wifi-next-button");
const mqttFinishButton = document.getElementById("mqtt-finish-button");
const finishButton = document.getElementById("finish-button");
const wifiSsidInput = document.getElementById("wifi-ssid");
const wifiPasswordInput = document.getElementById("wifi-password");
const mqttProtocolInput = document.getElementById("mqtt-protocol");
const mqttBrokerInput = document.getElementById("mqtt-broker");
const mqttUsernameInput = document.getElementById("mqtt-username");
const mqttPasswordInput = document.getElementById("mqtt-password");
const networkList = document.getElementById("network-list");
const scanStatus = document.getElementById("scan-status");
const wifiStatus = document.getElementById("wifi-status");
const mqttStatus = document.getElementById("mqtt-status");

document.querySelectorAll("[data-next]").forEach((button) => {
  button.addEventListener("click", () => setStep(state.currentStep + 1));
});

document.querySelectorAll("[data-prev]").forEach((button) => {
  button.addEventListener("click", () => setStep(state.currentStep - 1));
});

wifiForm.addEventListener("submit", async (event) => {
  event.preventDefault();

  const credentials = getWifiCredentials();
  if (!credentials) {
    return;
  }

  await startWifiFlow(credentials);
});

mqttForm.addEventListener("submit", async (event) => {
  event.preventDefault();

  const payload = getMqttPayload();
  if (!payload) {
    return;
  }

  await startMqttFlow(payload);
});

async function startMqttFlow(payload) {
  showStatus(
    mqttStatus,
    "H0m3 is preparing to connect to your broker.",
    "success",
  );

  loadingMessage.textContent = "Connecting H0m3 to your broker";
  loadingOverlay.hidden = false;
  mqttFinishButton.disabled = true;

  try {
    await apiRequest("/api/test-mqtt", {
      method: "POST",
      body: JSON.stringify(payload),
    });

    const status = await pollMqttStatus();
    showStatus(mqttStatus, status.message, "success");

    loadingMessage.textContent = "Saving broker settings";
    const response = await apiRequest("/api/save-mqtt", {
      method: "POST",
      body: JSON.stringify(payload),
    });

    showStatus(mqttStatus, response.message, "success");
    setStep(3);
  } catch (error) {
    showStatus(mqttStatus, error.message, "error");
  } finally {
    mqttFinishButton.disabled = false;
    loadingOverlay.hidden = true;
  }
}

finishButton.addEventListener("click", () => {
  window.close();
});

window.addEventListener("load", async () => {
  try {
    const payload = await apiRequest("/api/provisioning/status");
    scanStatus.textContent = `Setup network: ${payload.ap_ssid}`;
  } catch (error) {
    showStatus(wifiStatus, error.message, "error");
  }
});

function setStep(nextStep) {
  const safeStep = Math.max(0, Math.min(nextStep, stepViews.length - 1));
  state.currentStep = safeStep;

  stepViews.forEach((view, index) => {
    view.classList.toggle("is-active", index === safeStep);
  });

  stepIndicators.forEach((item, index) => {
    item.classList.toggle("is-active", index === safeStep);
  });

  if (safeStep === 1 && !state.hasLoadedWifiStep) {
    state.hasLoadedWifiStep = true;
    withLoading("Scanning for networks", loadCachedNetworks);
  }
}

async function loadCachedNetworks() {
  const payload = await apiRequest("/api/networks");
  renderNetworks(payload.networks || []);
  scanStatus.textContent = `${payload.networks.length} network(s) found`;
}

function renderNetworks(networks) {
  networkList.innerHTML = "";

  if (!networks.length) {
    networkList.innerHTML = '<p class="network-empty">No visible networks found.</p>';
    return;
  }

  networks.forEach((network) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "network-item";
    button.innerHTML = `
      <strong>${escapeHtml(network.ssid)}</strong>
      <div class="network-meta">Signal ${network.signal_strength} dBm · Channel ${network.channel} · ${network.auth_required ? "Secured" : "Open"}</div>
    `;

    button.addEventListener("click", () => {
      wifiSsidInput.value = network.ssid;

      document.querySelectorAll(".network-item").forEach((item) => {
        item.classList.remove("is-selected");
      });

      button.classList.add("is-selected");
    });

    networkList.appendChild(button);
  });
}

async function startWifiFlow(credentials) {
  showStatus(
    wifiStatus,
    "H0m3 is preparing to connect to your Wi-Fi.",
    "success",
  );

  loadingMessage.textContent = "Connecting H0m3 to your Wi-Fi";
  loadingOverlay.hidden = false;
  wifiNextButton.disabled = true;

  try {
    await apiRequest("/api/test-wifi", {
      method: "POST",
      body: JSON.stringify(credentials),
    });

    const status = await pollWifiStatus();
    showStatus(
      wifiStatus,
      `${status.message} ${status.ip ? `IP: ${status.ip}` : ""}`.trim(),
      "success",
    );

    loadingMessage.textContent = "Saving Wi-Fi settings";
    const saveResponse = await apiRequest("/api/save-wifi", {
      method: "POST",
      body: JSON.stringify(credentials),
    });

    showStatus(wifiStatus, saveResponse.message, "success");
    setStep(2);
  } catch (error) {
    showStatus(wifiStatus, error.message, "error");
  } finally {
    wifiNextButton.disabled = false;
    loadingOverlay.hidden = true;
  }
}

async function pollWifiStatus() {
  const deadline = Date.now() + 60_000;
  const reconnectHintAt = Date.now() + 10_000;

  while (Date.now() < deadline) {
    let status = null;

    try {
      status = await apiRequest("/api/wifi-status");
    } catch (_error) {
      // Temporary disconnects are expected while the ESP tests station mode.
    }

    if (status) {
      if (status.state === "scheduled" || status.state === "testing") {
        loadingMessage.textContent = status.message || "Connecting H0m3 to your Wi-Fi";
      } else if (status.state === "success") {
        return status;
      } else if (status.state === "error") {
        const codeSuffix = status.error_code != null ? ` (code ${status.error_code})` : "";
        throw new Error(`${status.message}${codeSuffix}`);
      }
    }

    if (Date.now() >= reconnectHintAt) {
      loadingMessage.textContent =
        "Still working. If your phone switched away, reconnect to the H0m3 setup network.";
    }

    await delay(200);
  }

  throw new Error("This is taking longer than expected. Please reconnect to the H0m3 setup network and try again.");
}

async function pollMqttStatus() {
  const deadline = Date.now() + 60_000;
  const reconnectHintAt = Date.now() + 10_000;

  while (Date.now() < deadline) {
    let status = null;

    try {
      status = await apiRequest("/api/mqtt-status");
    } catch (_error) {
      // Temporary disconnects are expected while the ESP reconnects Wi-Fi to reach the broker.
    }

    if (status) {
      if (status.state === "scheduled" || status.state === "testing") {
        loadingMessage.textContent = status.message || "Connecting H0m3 to your broker";
      } else if (status.state === "success") {
        return status;
      } else if (status.state === "error") {
        const codeSuffix = status.error_code != null ? ` (code ${status.error_code})` : "";
        throw new Error(`${status.message}${codeSuffix}`);
      }
    }

    if (Date.now() >= reconnectHintAt) {
      loadingMessage.textContent =
        "Still working. If your phone switched away, reconnect to the H0m3 setup network.";
    }

    await delay(200);
  }

  throw new Error("This is taking longer than expected. Please reconnect to the H0m3 setup network and try again.");
}

function getWifiCredentials() {
  const ssid = wifiSsidInput.value.trim();
  const password = wifiPasswordInput.value;

  if (!ssid) {
    showStatus(wifiStatus, "SSID is required before continuing.", "error");
    return null;
  }

  return { ssid, password };
}

function getMqttPayload() {
  const broker = mqttBrokerInput.value.trim();

  if (!broker) {
    showStatus(mqttStatus, "Broker is required before finishing.", "error");
    return null;
  }

  return {
    protocol: mqttProtocolInput.value,
    broker,
    username: mqttUsernameInput.value.trim(),
    password: mqttPasswordInput.value,
  };
}

function showStatus(element, message, type) {
  element.hidden = false;
  element.className = `status-banner ${type}`;
  element.textContent = message;
}

async function withLoading(message, action) {
  loadingMessage.textContent = message;
  loadingOverlay.hidden = false;

  try {
    await action();
  } catch (error) {
    showStatus(wifiStatus, error.message, "error");
  } finally {
    loadingOverlay.hidden = true;
  }
}

async function apiRequest(url, options = {}) {
  const response = await fetch(url, {
    headers: {
      "Content-Type": "application/json",
      ...(options.headers || {}),
    },
    ...options,
  });

  const payload = await response.json().catch(() => ({}));

  if (!response.ok) {
    throw new Error(payload.message || `Request failed with status ${response.status}`);
  }

  return payload;
}

function delay(ms) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}
