//! `bench/secrets.toml`: Wi-Fi and MQTT settings of the network stand. They are put into the
//! stand project only; nothing here ever reaches the report or the console.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

#[derive(Default, Deserialize)]
pub struct Secrets {
    #[serde(default)]
    wifi: Wifi,
    #[serde(default)]
    mqtt: Mqtt,
}

#[derive(Default, Deserialize)]
struct Wifi {
    #[serde(default)]
    ssid: String,
    #[serde(default)]
    password: String,
}

#[derive(Default, Deserialize)]
struct Mqtt {
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    user: String,
    #[serde(default)]
    password: String,
}

/// Complete settings of the network stand.
pub struct Net {
    pub ssid: String,
    pub wifi_password: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub mqtt_password: String,
}

/// Wi-Fi settings alone (the Modbus stand needs no broker).
pub struct WifiSettings {
    pub ssid: String,
    pub password: String,
}

impl Secrets {
    pub fn parse(text: &str) -> Result<Self, String> {
        // The parser error may quote the line; it is not echoed.
        toml::from_str(text)
            .map_err(|_| "не разобран bench/secrets.toml: неверный формат".to_owned())
    }

    /// `None` when the file is absent; an error when it is there but unreadable.
    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("не прочитан {}: {}", path.display(), e.kind())),
        }
    }

    /// The Wi-Fi settings, if the file names a network.
    pub fn wifi(&self) -> Option<WifiSettings> {
        if self.wifi.ssid.is_empty() {
            return None;
        }
        Some(WifiSettings {
            ssid: self.wifi.ssid.clone(),
            password: self.wifi.password.clone(),
        })
    }

    /// The settings of the network stand, if the file names a network and a broker.
    pub fn net(&self) -> Option<Net> {
        if self.wifi.ssid.is_empty() || self.mqtt.host.is_empty() {
            return None;
        }
        Some(Net {
            ssid: self.wifi.ssid.clone(),
            wifi_password: self.wifi.password.clone(),
            host: self.mqtt.host.clone(),
            port: self.mqtt.port.unwrap_or(1883),
            user: self.mqtt.user.clone(),
            mqtt_password: self.mqtt.password.clone(),
        })
    }
}

impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secrets(..)")
    }
}
