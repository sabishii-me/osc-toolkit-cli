//! Versioned configuration model and semantic validation.

use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    net::SocketAddr,
    path::Path,
};

use serde::Deserialize;
use thiserror::Error;

/// The currently supported configuration schema version.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

/// A stable, user-defined identifier used to reference configuration objects.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(transparent)]
pub struct ResourceId(String);

impl ResourceId {
    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Top-level configuration file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Schema version. Only version 1 is currently accepted.
    pub version: u32,
    /// Local UDP listeners.
    #[serde(default)]
    pub inputs: Vec<InputConfig>,
    /// Remote UDP destinations.
    #[serde(default)]
    pub outputs: Vec<OutputConfig>,
    /// Explicit input-to-output edges.
    #[serde(default)]
    pub mappings: Vec<MappingConfig>,
    /// Concise declarations that expand into mapping edges.
    #[serde(default)]
    pub mapping_groups: Vec<MappingGroupConfig>,
}

/// Input payload handling mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum InputMode {
    /// Forward UDP datagrams without OSC decoding.
    #[default]
    Passthrough,
    /// Validate and optionally process OSC datagrams.
    Osc,
}

/// Local UDP listener configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputConfig {
    /// Unique input identifier.
    pub id: ResourceId,
    /// Local socket address to bind.
    pub bind: SocketAddr,
    /// Payload handling mode.
    #[serde(default)]
    pub mode: InputMode,
}

/// Remote UDP destination configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    /// Unique output identifier.
    pub id: ResourceId,
    /// Remote socket address.
    pub target: SocketAddr,
}

/// One explicit directed mapping edge.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingConfig {
    /// Unique mapping identifier.
    pub id: ResourceId,
    /// Referenced input identifier.
    pub from: ResourceId,
    /// Referenced output identifier.
    pub to: ResourceId,
}

/// Supported group expansion modes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MappingGroupMode {
    /// Create the Cartesian product of source inputs and destination outputs.
    AllToAll,
    /// Pair sources and destinations by index.
    Zip,
}

/// A concise declaration for multiple mapping edges.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingGroupConfig {
    /// Unique group identifier.
    pub id: ResourceId,
    /// Source input identifiers.
    pub from: Vec<ResourceId>,
    /// Destination output identifiers.
    pub to: Vec<ResourceId>,
    /// Expansion strategy.
    pub mode: MappingGroupMode,
}

/// One fully resolved mapping edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMapping {
    /// Stable edge identifier. Group-generated identifiers are deterministic.
    pub id: String,
    /// Zero-based input index in [`ValidatedConfig::inputs`].
    pub input_index: usize,
    /// Zero-based output index in [`ValidatedConfig::outputs`].
    pub output_index: usize,
}

/// A semantically valid, resolved runtime configuration.
#[derive(Debug)]
pub struct ValidatedConfig {
    /// Input definitions in declaration order.
    pub inputs: Vec<InputConfig>,
    /// Output definitions in declaration order.
    pub outputs: Vec<OutputConfig>,
    /// Explicit and group-generated edges in deterministic order.
    pub mappings: Vec<ResolvedMapping>,
}

/// A configuration loading or validation failure.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Reading a configuration file failed.
    #[error("failed to read configuration file `{path}`: {source}")]
    Read {
        /// Requested file path.
        path: String,
        /// Underlying file-system error.
        source: std::io::Error,
    },
    /// TOML deserialization failed.
    #[error("invalid TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),
    /// The schema version is not supported.
    #[error("unsupported configuration version {found}; expected {expected}")]
    UnsupportedVersion {
        /// Version found in the configuration.
        found: u32,
        /// Version supported by this binary.
        expected: u32,
    },
    /// An identifier is empty or contains unsupported characters.
    #[error("invalid {kind} identifier `{id}`; use ASCII letters, digits, `.`, `_`, or `-`")]
    InvalidId {
        /// Resource category.
        kind: &'static str,
        /// Invalid identifier.
        id: String,
    },
    /// An identifier appears more than once in its namespace.
    #[error("duplicate {kind} identifier `{id}`")]
    DuplicateId {
        /// Resource category.
        kind: &'static str,
        /// Repeated identifier.
        id: String,
    },
    /// A mapping references an unknown input.
    #[error("{owner} references unknown input `{id}`")]
    UnknownInput {
        /// Mapping or group that owns the reference.
        owner: String,
        /// Missing input identifier.
        id: String,
    },
    /// A mapping references an unknown output.
    #[error("{owner} references unknown output `{id}`")]
    UnknownOutput {
        /// Mapping or group that owns the reference.
        owner: String,
        /// Missing output identifier.
        id: String,
    },
    /// A group has no source inputs or destination outputs.
    #[error("mapping group `{id}` must contain at least one input and one output")]
    EmptyMappingGroup {
        /// Invalid group identifier.
        id: String,
    },
    /// A zip group has different source and destination counts.
    #[error(
        "mapping group `{id}` uses zip mode but has {input_count} inputs and {output_count} outputs"
    )]
    ZipLengthMismatch {
        /// Invalid group identifier.
        id: String,
        /// Number of source inputs.
        input_count: usize,
        /// Number of destination outputs.
        output_count: usize,
    },
    /// Two declarations resolve to the same input-output edge.
    #[error(
        "duplicate mapping edge `{from}` -> `{to}` generated by `{duplicate}`; first declared by `{first}`"
    )]
    DuplicateEdge {
        /// Input identifier.
        from: String,
        /// Output identifier.
        to: String,
        /// First edge owner.
        first: String,
        /// Duplicate edge owner.
        duplicate: String,
    },
}

/// Loads, parses, and validates a configuration file.
///
/// # Errors
///
/// Returns [`ConfigError`] when the file cannot be read, TOML is invalid, or
/// semantic validation fails.
pub fn load_and_validate(path: impl AsRef<Path>) -> Result<ValidatedConfig, ConfigError> {
    let path = path.as_ref();
    let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    parse_and_validate(&source)
}

/// Parses and validates a TOML configuration string.
///
/// # Errors
///
/// Returns [`ConfigError`] when TOML is invalid or semantic validation fails.
pub fn parse_and_validate(source: &str) -> Result<ValidatedConfig, ConfigError> {
    let config: Config = toml::from_str(source)?;
    config.validate()
}

impl Config {
    /// Resolves references and mapping groups into a validated runtime model.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for unsupported versions, invalid identifiers,
    /// unknown references, invalid groups, and duplicate mapping edges.
    pub fn validate(self) -> Result<ValidatedConfig, ConfigError> {
        if self.version != CURRENT_CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                found: self.version,
                expected: CURRENT_CONFIG_VERSION,
            });
        }

        let input_ids = index_resources(self.inputs.iter().map(|input| &input.id), "input")?;
        let output_ids = index_resources(self.outputs.iter().map(|output| &output.id), "output")?;

        let mut mapping_ids = HashSet::new();
        for mapping in &self.mappings {
            validate_and_insert_id(&mapping.id, "mapping", &mut mapping_ids)?;
        }

        let mut group_ids = HashSet::new();
        for group in &self.mapping_groups {
            validate_and_insert_id(&group.id, "mapping group", &mut group_ids)?;
            if mapping_ids.contains(group.id.as_str()) {
                return Err(ConfigError::DuplicateId {
                    kind: "mapping or mapping group",
                    id: group.id.to_string(),
                });
            }
        }

        let mut mappings = Vec::new();
        let mut edges: HashMap<(usize, usize), String> = HashMap::new();

        for mapping in &self.mappings {
            let input_index = resolve_input(&input_ids, &mapping.from, mapping.id.as_str())?;
            let output_index = resolve_output(&output_ids, &mapping.to, mapping.id.as_str())?;
            push_edge(
                &mut mappings,
                &mut edges,
                mapping.id.to_string(),
                input_index,
                output_index,
                &self.inputs,
                &self.outputs,
            )?;
        }

        for group in &self.mapping_groups {
            if group.from.is_empty() || group.to.is_empty() {
                return Err(ConfigError::EmptyMappingGroup {
                    id: group.id.to_string(),
                });
            }

            let inputs = group
                .from
                .iter()
                .map(|id| resolve_input(&input_ids, id, group.id.as_str()))
                .collect::<Result<Vec<_>, _>>()?;
            let outputs = group
                .to
                .iter()
                .map(|id| resolve_output(&output_ids, id, group.id.as_str()))
                .collect::<Result<Vec<_>, _>>()?;

            match group.mode {
                MappingGroupMode::AllToAll => {
                    for (source_position, &input_index) in inputs.iter().enumerate() {
                        for (target_position, &output_index) in outputs.iter().enumerate() {
                            push_edge(
                                &mut mappings,
                                &mut edges,
                                format!("{}[{source_position},{target_position}]", group.id),
                                input_index,
                                output_index,
                                &self.inputs,
                                &self.outputs,
                            )?;
                        }
                    }
                }
                MappingGroupMode::Zip => {
                    if inputs.len() != outputs.len() {
                        return Err(ConfigError::ZipLengthMismatch {
                            id: group.id.to_string(),
                            input_count: inputs.len(),
                            output_count: outputs.len(),
                        });
                    }
                    for (position, (&input_index, &output_index)) in
                        inputs.iter().zip(&outputs).enumerate()
                    {
                        push_edge(
                            &mut mappings,
                            &mut edges,
                            format!("{}[{position}]", group.id),
                            input_index,
                            output_index,
                            &self.inputs,
                            &self.outputs,
                        )?;
                    }
                }
            }
        }

        Ok(ValidatedConfig {
            inputs: self.inputs,
            outputs: self.outputs,
            mappings,
        })
    }
}

fn index_resources<'a>(
    ids: impl Iterator<Item = &'a ResourceId>,
    kind: &'static str,
) -> Result<HashMap<String, usize>, ConfigError> {
    let mut index = HashMap::new();
    for (position, id) in ids.enumerate() {
        validate_id(id, kind)?;
        if index.insert(id.to_string(), position).is_some() {
            return Err(ConfigError::DuplicateId {
                kind,
                id: id.to_string(),
            });
        }
    }
    Ok(index)
}

fn validate_and_insert_id(
    id: &ResourceId,
    kind: &'static str,
    seen: &mut HashSet<String>,
) -> Result<(), ConfigError> {
    validate_id(id, kind)?;
    if !seen.insert(id.to_string()) {
        return Err(ConfigError::DuplicateId {
            kind,
            id: id.to_string(),
        });
    }
    Ok(())
}

fn validate_id(id: &ResourceId, kind: &'static str) -> Result<(), ConfigError> {
    let valid = !id.as_str().is_empty()
        && id
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ConfigError::InvalidId {
            kind,
            id: id.to_string(),
        })
    }
}

fn resolve_input(
    inputs: &HashMap<String, usize>,
    id: &ResourceId,
    owner: &str,
) -> Result<usize, ConfigError> {
    inputs
        .get(id.as_str())
        .copied()
        .ok_or_else(|| ConfigError::UnknownInput {
            owner: owner.to_owned(),
            id: id.to_string(),
        })
}

fn resolve_output(
    outputs: &HashMap<String, usize>,
    id: &ResourceId,
    owner: &str,
) -> Result<usize, ConfigError> {
    outputs
        .get(id.as_str())
        .copied()
        .ok_or_else(|| ConfigError::UnknownOutput {
            owner: owner.to_owned(),
            id: id.to_string(),
        })
}

#[allow(clippy::too_many_arguments)]
fn push_edge(
    mappings: &mut Vec<ResolvedMapping>,
    edges: &mut HashMap<(usize, usize), String>,
    id: String,
    input_index: usize,
    output_index: usize,
    inputs: &[InputConfig],
    outputs: &[OutputConfig],
) -> Result<(), ConfigError> {
    if let Some(first) = edges.insert((input_index, output_index), id.clone()) {
        return Err(ConfigError::DuplicateEdge {
            from: inputs[input_index].id.to_string(),
            to: outputs[output_index].id.to_string(),
            first,
            duplicate: id,
        });
    }
    mappings.push(ResolvedMapping {
        id,
        input_index,
        output_index,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, InputMode, parse_and_validate};

    const BASE: &str = r#"
version = 1

[[inputs]]
id = "face-a"
bind = "0.0.0.0:11111"

[[inputs]]
id = "face-b"
bind = "[::]:11112"
mode = "osc"

[[outputs]]
id = "unreal-a"
target = "127.0.0.1:12001"

[[outputs]]
id = "unreal-b"
target = "127.0.0.1:12002"
"#;

    #[test]
    fn parses_defaults_and_ipv6() {
        let config = parse_and_validate(BASE).expect("configuration should be valid");
        assert_eq!(config.inputs[0].mode, InputMode::Passthrough);
        assert_eq!(config.inputs[1].mode, InputMode::Osc);
    }

    #[test]
    fn expands_all_to_all_deterministically() {
        let source = format!(
            "{BASE}\n[[mapping_groups]]\nid = \"faces\"\nfrom = [\"face-a\", \"face-b\"]\nto = [\"unreal-a\", \"unreal-b\"]\nmode = \"all-to-all\"\n"
        );
        let config = parse_and_validate(&source).expect("configuration should be valid");
        let edges = config
            .mappings
            .iter()
            .map(|edge| (edge.input_index, edge.output_index, edge.id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            edges,
            vec![
                (0, 0, "faces[0,0]"),
                (0, 1, "faces[0,1]"),
                (1, 0, "faces[1,0]"),
                (1, 1, "faces[1,1]"),
            ]
        );
    }

    #[test]
    fn expands_zip_by_position() {
        let source = format!(
            "{BASE}\n[[mapping_groups]]\nid = \"faces\"\nfrom = [\"face-a\", \"face-b\"]\nto = [\"unreal-b\", \"unreal-a\"]\nmode = \"zip\"\n"
        );
        let config = parse_and_validate(&source).expect("configuration should be valid");
        assert_eq!(
            config.mappings,
            vec![
                super::ResolvedMapping {
                    id: "faces[0]".to_owned(),
                    input_index: 0,
                    output_index: 1,
                },
                super::ResolvedMapping {
                    id: "faces[1]".to_owned(),
                    input_index: 1,
                    output_index: 0,
                },
            ]
        );
    }

    #[test]
    fn rejects_zip_length_mismatch() {
        let source = format!(
            "{BASE}\n[[mapping_groups]]\nid = \"faces\"\nfrom = [\"face-a\"]\nto = [\"unreal-a\", \"unreal-b\"]\nmode = \"zip\"\n"
        );
        assert!(matches!(
            parse_and_validate(&source),
            Err(ConfigError::ZipLengthMismatch { .. })
        ));
    }

    #[test]
    fn rejects_unknown_references() {
        let source = format!(
            "{BASE}\n[[mappings]]\nid = \"missing\"\nfrom = \"unknown\"\nto = \"unreal-a\"\n"
        );
        assert!(matches!(
            parse_and_validate(&source),
            Err(ConfigError::UnknownInput { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_edges_from_different_declarations() {
        let source = format!(
            "{BASE}\n[[mappings]]\nid = \"direct\"\nfrom = \"face-a\"\nto = \"unreal-a\"\n\n[[mapping_groups]]\nid = \"duplicate\"\nfrom = [\"face-a\"]\nto = [\"unreal-a\"]\nmode = \"all-to-all\"\n"
        );
        assert!(matches!(
            parse_and_validate(&source),
            Err(ConfigError::DuplicateEdge { .. })
        ));
    }

    #[test]
    fn rejects_unknown_fields() {
        let source = "version = 1\nunexpected = true\n";
        assert!(matches!(
            parse_and_validate(source),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn rejects_non_ascii_identifier() {
        let source = "version = 1\n[[inputs]]\nid = \"face/one\"\nbind = \"127.0.0.1:1\"\n";
        assert!(matches!(
            parse_and_validate(source),
            Err(ConfigError::InvalidId { .. })
        ));
    }
}
