//! Finding the microphones and outputs the system has, from `pw-dump`.

use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Source,
    Sink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// The PipeWire node name, which is what is passed back to use it.
    pub name: String,
    pub description: String,
    pub kind: DeviceKind,
}

/// The real microphones and outputs in a `pw-dump` listing. Our own virtual
/// nodes are left out (a microphone must not be fed from itself), and so are
/// monitors and other virtual devices.
pub fn parse_devices(dump: &str) -> Vec<AudioDevice> {
    let Ok(Value::Array(objects)) = serde_json::from_str::<Value>(dump) else {
        return Vec::new();
    };
    let mut devices: Vec<AudioDevice> = objects
        .iter()
        .filter_map(|object| {
            let props = object.get("info")?.get("props")?;
            let kind = match props.get("media.class")?.as_str()? {
                "Audio/Source" => DeviceKind::Source,
                "Audio/Sink" => DeviceKind::Sink,
                _ => return None,
            };
            let name = props.get("node.name")?.as_str()?.to_owned();
            if name.starts_with("lumencast_") {
                return None;
            }
            let description = props
                .get("node.description")
                .or_else(|| props.get("node.nick"))
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_owned();
            Some(AudioDevice {
                name,
                description,
                kind,
            })
        })
        .collect();
    devices.sort_by(|a, b| (a.kind as u8, &a.description).cmp(&(b.kind as u8, &b.description)));
    devices
}

/// Asks PipeWire for its devices.
pub fn list_devices() -> Result<Vec<AudioDevice>, String> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|error| format!("pw-dump could not be run (is PipeWire installed?): {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "pw-dump failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(parse_devices(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"[
      {"id": 46, "type": "PipeWire:Interface:Device", "info": {"props": {"media.class": "Audio/Device"}}},
      {"id": 56, "info": {"props": {"media.class": "Audio/Sink", "node.name": "alsa_output.usb-Fractal", "node.description": "Fractal Scape Dongle"}}},
      {"id": 57, "info": {"props": {"media.class": "Audio/Source", "node.name": "alsa_input.usb-Fractal", "node.description": "Fractal Mono"}}},
      {"id": 67, "info": {"props": {"media.class": "Audio/Source", "node.name": "alsa_input.pci", "node.nick": "Ryzen"}}},
      {"id": 89, "info": {"props": {"media.class": "Audio/Source/Virtual", "node.name": "lumencast_microphone"}}},
      {"id": 90, "info": {"props": {"media.class": "Audio/Sink", "node.name": "lumencast_speakers", "node.description": "LumenCast Speakers"}}},
      {"id": 91, "info": {"props": {"media.class": "Stream/Output/Audio", "node.name": "Chromium"}}},
      {"id": 92, "type": "x"}
    ]"#;

    #[test]
    fn real_microphones_and_outputs_are_listed_and_ours_and_the_rest_are_not() {
        let devices = parse_devices(DUMP);
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "alsa_input.usb-Fractal",
                "alsa_input.pci",
                "alsa_output.usb-Fractal"
            ]
        );
        assert_eq!(devices.len(), 3);
        assert!(devices.iter().all(|d| !d.name.starts_with("lumencast_")));
    }

    #[test]
    fn a_description_falls_back_to_the_nick_and_then_the_name() {
        let devices = parse_devices(DUMP);
        let pci = devices.iter().find(|d| d.name == "alsa_input.pci").unwrap();
        assert_eq!(pci.description, "Ryzen");
        let bare =
            parse_devices(r#"[{"info":{"props":{"media.class":"Audio/Source","node.name":"x"}}}]"#);
        assert_eq!(bare[0].description, "x");
    }

    #[test]
    fn sources_come_before_sinks_and_junk_gives_an_empty_list() {
        let devices = parse_devices(DUMP);
        assert_eq!(devices[0].kind, DeviceKind::Source);
        assert_eq!(devices.last().unwrap().kind, DeviceKind::Sink);
        assert!(parse_devices("not json").is_empty());
        assert!(parse_devices("{}").is_empty());
    }
}
