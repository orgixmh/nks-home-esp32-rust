use std::sync::{Arc, Mutex};
use std::time::Duration;

use esp_idf_svc::mqtt::client::{
    EspMqttClient, EventPayload, MessageId, MqttClientConfiguration, QoS,
};
use log::{info, warn};

use crate::config::types::MqttConfig;
use crate::error::AppError;

type MessageHandler = Arc<dyn Fn(&MqttMessage) + Send + Sync + 'static>;

pub struct MqttManager {
    client: EspMqttClient<'static>,
    state: Arc<Mutex<MqttState>>,
}

struct MqttState {
    connected: bool,
    last_error: Option<String>,
    subscriptions: Vec<Subscription>,
}

struct Subscription {
    topic_filter: String,
    handler: MessageHandler,
}

pub struct MqttMessage {
    pub topic: String,
    pub payload: Vec<u8>,
}

impl MqttManager {
    pub fn connect(config: &MqttConfig) -> Result<Self, AppError> {
        let state = Arc::new(Mutex::new(MqttState {
            connected: false,
            last_error: None,
            subscriptions: Vec::new(),
        }));
        let event_state = state.clone();
        let broker_url = broker_url(config);

        let client = EspMqttClient::new_cb(
            &broker_url,
            &MqttClientConfiguration {
                client_id: Some(&config.client_id),
                username: optional_str(&config.username),
                password: optional_str(&config.password),
                keep_alive_interval: Some(Duration::from_secs(30)),
                reconnect_timeout: Some(Duration::from_secs(10)),
                network_timeout: Duration::from_secs(10),
                ..Default::default()
            },
            move |event| match event.payload() {
                EventPayload::Connected(_) => {
                    if let Ok(mut state) = event_state.lock() {
                        state.connected = true;
                        state.last_error = None;
                    }

                    info!("MQTT connected");
                }
                EventPayload::Disconnected => {
                    if let Ok(mut state) = event_state.lock() {
                        state.connected = false;
                    }

                    warn!("MQTT disconnected");
                }
                EventPayload::Received {
                    topic,
                    data,
                    details,
                    ..
                } => {
                    if !matches!(details, esp_idf_svc::mqtt::client::Details::Complete) {
                        warn!("Skipping chunked MQTT message");
                        return;
                    }

                    let Some(topic) = topic else {
                        return;
                    };

                    let message = MqttMessage {
                        topic: topic.to_string(),
                        payload: data.to_vec(),
                    };

                    let handlers = if let Ok(state) = event_state.lock() {
                        state
                            .subscriptions
                            .iter()
                            .filter(|subscription| {
                                topic_matches(&subscription.topic_filter, &message.topic)
                            })
                            .map(|subscription| subscription.handler.clone())
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };

                    for handler in handlers {
                        handler(&message);
                    }
                }
                EventPayload::Error(error) => {
                    if let Ok(mut state) = event_state.lock() {
                        state.last_error = Some(format!("MQTT connection error: {error:?}"));
                    }

                    warn!("MQTT event error: {error:?}");
                }
                _ => {}
            },
        )?;

        Ok(Self { client, state })
    }

    pub fn publish(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: QoS,
        retain: bool,
    ) -> Result<MessageId, AppError> {
        Ok(self.client.publish(topic, qos, retain, payload)?)
    }

    pub fn subscribe<F>(
        &mut self,
        topic_filter: &str,
        qos: QoS,
        handler: F,
    ) -> Result<MessageId, AppError>
    where
        F: Fn(&MqttMessage) + Send + Sync + 'static,
    {
        let message_id = self.client.subscribe(topic_filter, qos)?;
        self.state
            .lock()
            .map_err(|_| AppError::Message("MQTT state lock poisoned".into()))?
            .subscriptions
            .push(Subscription {
                topic_filter: topic_filter.to_string(),
                handler: Arc::new(handler),
            });

        Ok(message_id)
    }

    pub fn is_connected(&self) -> Result<bool, AppError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| AppError::Message("MQTT state lock poisoned".into()))?
            .connected)
    }

    pub fn wait_until_connected(&self, timeout: Duration) -> Result<(), AppError> {
        let deadline = std::time::Instant::now() + timeout;

        while std::time::Instant::now() < deadline {
            let state = self
                .state
                .lock()
                .map_err(|_| AppError::Message("MQTT state lock poisoned".into()))?;

            if state.connected {
                return Ok(());
            }

            if let Some(error) = &state.last_error {
                return Err(AppError::Message(error.clone()));
            }

            drop(state);
            std::thread::sleep(Duration::from_millis(200));
        }

        Err(AppError::Message(
            "Unable to reach your broker. Please check the MQTT details and try again.".into(),
        ))
    }
}

pub fn test_connection(config: &MqttConfig, timeout: Duration) -> Result<(), AppError> {
    let manager = MqttManager::connect(config)?;
    let result = manager.wait_until_connected(timeout);
    drop(manager);
    result
}

fn broker_url(config: &MqttConfig) -> String {
    let scheme = if config.port == 8883 { "mqtts" } else { "mqtt" };
    format!("{scheme}://{}:{}", config.host, config.port)
}

fn optional_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn topic_matches(filter: &str, topic: &str) -> bool {
    let filter_levels: Vec<&str> = filter.split('/').collect();
    let topic_levels: Vec<&str> = topic.split('/').collect();
    let mut topic_index = 0usize;

    for (index, filter_level) in filter_levels.iter().enumerate() {
        match *filter_level {
            "#" => return index == filter_levels.len() - 1,
            "+" => {
                if topic_index >= topic_levels.len() {
                    return false;
                }

                topic_index += 1;
            }
            _ => {
                if topic_levels.get(topic_index) != Some(filter_level) {
                    return false;
                }

                topic_index += 1;
            }
        }
    }

    topic_index == topic_levels.len()
}
