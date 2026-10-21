//! The lab file: one TOML file per lab, designed to diff cleanly in git.
//!
//! Devices and links are flat lists. Device configs are stored as IOS-style text so a diff of
//! a lab reads like a config change. Unknown keys are preserved on load and written back, so
//! a newer lab survives a round trip through an older build (with a warning).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Placeholder until the project is renamed.
pub const FORMAT: &str = "labsim-lab";
/// Newest version this build understands. Older versions are migrated on load.
pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabFile {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub meta: Meta,
    #[serde(rename = "device", default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Device>,
    #[serde(rename = "link", default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(rename = "annotation", default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Keys this build does not know: kept, warned about, and written back.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Root RNG seed; part of determinism.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    /// Our own template ids (`router-2g`, `host-pc`, `switch-l2-8`), never vendor model numbers.
    pub template: String,
    /// Canvas position in logical units. Integers keep the output float-free and stable.
    pub pos: [i32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_config: Option<String>,
    /// Present only when it differs from the startup config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_config: Option<String>,
    /// Structured settings for hosts, which have no IOS-style config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<HostConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostConfig {
    /// `a.b.c.d/len`
    pub ipv4: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// `Device:port`
    pub a: String,
    pub b: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub text: String,
    pub pos: [i32; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    Parse(String),
    WrongFormat(String),
    /// The file is newer than this build.
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Parse(e) => write!(f, "not a valid lab file: {e}"),
            LoadError::WrongFormat(got) => write!(f, "not a lab file (format = {got:?})"),
            LoadError::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "lab file version {found} is newer than this build supports ({supported})"
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// A loaded lab plus non-fatal findings (unknown keys, duplicate names).
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub lab: LabFile,
    pub warnings: Vec<String>,
}

/// Parse, check the header and migrate to the current version.
pub fn load(text: &str) -> Result<Loaded, LoadError> {
    let lab: LabFile = toml::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
    if lab.format != FORMAT {
        return Err(LoadError::WrongFormat(lab.format));
    }
    if lab.version > CURRENT_VERSION {
        return Err(LoadError::UnsupportedVersion {
            found: lab.version,
            supported: CURRENT_VERSION,
        });
    }
    // Migrations v1..=CURRENT would run here; there is only v1 so far.
    let mut warnings = Vec::new();
    for k in lab.unknown.keys() {
        warnings.push(format!("unknown top-level key `{k}` kept but ignored"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for d in &lab.devices {
        if !seen.insert(d.name.as_str()) {
            warnings.push(format!("duplicate device name `{}`", d.name));
        }
    }
    Ok(Loaded { lab, warnings })
}

/// Canonical output: fixed key order (struct order), integer-only numbers, optional fields
/// omitted. Saving an unchanged lab gives byte-identical text.
pub fn save(lab: &LabFile) -> String {
    toml::to_string(lab).expect("lab files are always serialisable")
}

/// Split `Device:port` into its parts.
pub fn split_endpoint(s: &str) -> Option<(&str, &str)> {
    let (d, p) = s.split_once(':')?;
    (!d.is_empty() && !p.is_empty()).then_some((d, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"format  = "labsim-lab"
version = 1

[meta]
title  = "VLANs and a trunk"
author = "A. Instructor"
seed   = 42

[[device]]
name     = "SW1"
template = "switch-l2-24"
pos      = [320, 140]
startup_config = '''
hostname SW1
!
vlan 10
 name STAFF
'''

[[device]]
name     = "PC1"
template = "host-pc"
pos      = [120, 80]
[device.host]
ipv4    = "192.168.10.10/24"
gateway = "192.168.10.1"
dns     = ["192.168.10.5"]

[[link]]
a      = "PC1:eth0"
b      = "SW1:Fa0/1"
medium = "copper"

[[annotation]]
text = "Trunk carries VLANs 10 and 20"
pos  = [400, 60]
"#;

    #[test]
    fn loads_the_documented_example() {
        let l = load(EXAMPLE).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert_eq!(l.lab.meta.title, "VLANs and a trunk");
        assert_eq!(l.lab.meta.seed, Some(42));
        assert_eq!(l.lab.devices.len(), 2);
        assert_eq!(l.lab.devices[0].pos, [320, 140]);
        assert!(l.lab.devices[0]
            .startup_config
            .as_deref()
            .unwrap()
            .contains("vlan 10"));
        assert_eq!(
            l.lab.devices[1].host.as_ref().unwrap().ipv4,
            "192.168.10.10/24"
        );
        assert_eq!(l.lab.links[0].b, "SW1:Fa0/1");
        assert_eq!(l.lab.annotations[0].pos, [400, 60]);
    }

    #[test]
    fn load_save_load_is_lossless() {
        let a = load(EXAMPLE).unwrap().lab;
        let b = load(&save(&a)).unwrap().lab;
        assert_eq!(a, b);
    }

    #[test]
    fn saving_an_unchanged_lab_is_a_zero_line_diff() {
        let once = save(&load(EXAMPLE).unwrap().lab);
        let twice = save(&load(&once).unwrap().lab);
        assert_eq!(once, twice, "canonical output must be a fixed point");
    }

    #[test]
    fn config_text_survives_exactly() {
        let a = load(EXAMPLE).unwrap().lab;
        let b = load(&save(&a)).unwrap().lab;
        assert_eq!(a.devices[0].startup_config, b.devices[0].startup_config);
        assert!(
            save(&a).contains("hostname SW1"),
            "config stays readable in the file"
        );
    }

    #[test]
    fn unknown_keys_are_kept_warned_and_written_back() {
        let text = format!("{EXAMPLE}\n[future_feature]\nenabled = true\n");
        let l = load(&text).unwrap();
        assert_eq!(l.warnings.len(), 1);
        assert!(l.warnings[0].contains("future_feature"));
        assert!(save(&l.lab).contains("future_feature"));
    }

    #[test]
    fn newer_versions_and_foreign_files_are_rejected() {
        let newer = EXAMPLE.replace("version = 1", "version = 99");
        assert_eq!(
            load(&newer).unwrap_err(),
            LoadError::UnsupportedVersion {
                found: 99,
                supported: CURRENT_VERSION
            }
        );
        let other = EXAMPLE.replace("labsim-lab", "something-else");
        assert!(matches!(load(&other), Err(LoadError::WrongFormat(_))));
        assert!(matches!(
            load("this is not toml ["),
            Err(LoadError::Parse(_))
        ));
        assert!(
            matches!(load("version = 1"), Err(LoadError::Parse(_))),
            "format is required"
        );
    }

    #[test]
    fn duplicate_device_names_warn() {
        let text = EXAMPLE.replace("name     = \"PC1\"", "name     = \"SW1\"");
        let l = load(&text).unwrap();
        assert!(l
            .warnings
            .iter()
            .any(|w| w.contains("duplicate device name `SW1`")));
    }

    #[test]
    fn endpoints_split_on_the_first_colon() {
        assert_eq!(split_endpoint("PC1:eth0"), Some(("PC1", "eth0")));
        assert_eq!(split_endpoint("noport"), None);
        assert_eq!(split_endpoint(":x"), None);
    }

    #[test]
    fn minimal_file_is_valid() {
        let l = load("format = \"labsim-lab\"\nversion = 1\n").unwrap();
        assert!(l.lab.devices.is_empty() && l.lab.links.is_empty());
    }
}
