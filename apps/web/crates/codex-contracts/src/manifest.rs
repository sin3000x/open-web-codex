use std::collections::HashMap;

use thiserror::Error;

use crate::generated::{CapabilityDeclaration, CapabilityManifest, CapabilityStatus};

#[derive(Debug, Clone, PartialEq)]
pub struct NegotiationPolicy {
    pub client_protocol_version: String,
    pub allowed_server_builds: Vec<String>,
    pub required_capabilities: Vec<String>,
    pub allow_experimental: bool,
    pub allow_experimental_ids: Vec<String>,
}

impl Default for NegotiationPolicy {
    fn default() -> Self {
        Self {
            client_protocol_version: "1.0.0".to_string(),
            allowed_server_builds: Vec::new(),
            required_capabilities: Vec::new(),
            allow_experimental: false,
            allow_experimental_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NegotiatedCapability {
    pub declaration: CapabilityDeclaration,
    pub effective_status: CapabilityStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NegotiationResult {
    pub status: &'static str,
    pub reasons: Vec<String>,
    pub capabilities: HashMap<String, NegotiatedCapability>,
}

#[derive(Debug, Error, PartialEq)]
pub enum ManifestError {
    #[error("unsupported Capability Manifest schema major {0}")]
    UnsupportedSchemaMajor(u32),
    #[error("{0} must be SemVer")]
    InvalidSemVer(&'static str),
    #[error("capabilities must be an array")]
    CapabilitiesNotArray,
    #[error("capability id is invalid")]
    InvalidCapabilityId,
    #[error("duplicate capability id {0}")]
    DuplicateCapabilityId(String),
    #[error("{0} has unknown status")]
    UnknownStatus(&'static str),
    #[error("{0} requires a structured reason")]
    MissingReason(String),
}

pub fn parse_capability_manifest(
    manifest: CapabilityManifest,
) -> Result<(CapabilityManifest, HashMap<String, CapabilityDeclaration>), ManifestError> {
    let schema_major = parse_version(&manifest.schema_version, "schemaVersion")?[0];
    if schema_major != 1 {
        return Err(ManifestError::UnsupportedSchemaMajor(schema_major));
    }
    parse_version(&manifest.server.protocol_version, "server.protocolVersion")?;
    parse_version(
        &manifest.compatibility.minimum_client_protocol,
        "minimumClientProtocol",
    )?;
    parse_version(
        &manifest.compatibility.maximum_client_protocol,
        "maximumClientProtocol",
    )?;

    let mut by_id = HashMap::new();
    for capability in manifest.capabilities.clone() {
        if !capability.id.contains('.') {
            return Err(ManifestError::InvalidCapabilityId);
        }
        if by_id.contains_key(&capability.id) {
            return Err(ManifestError::DuplicateCapabilityId(capability.id.clone()));
        }
        parse_version(&capability.version, "capability.version")?;
        validate_status(&capability)?;
        by_id.insert(capability.id.clone(), capability);
    }

    Ok((manifest, by_id))
}

pub fn negotiate_capability_manifest(
    manifest: CapabilityManifest,
    policy: &NegotiationPolicy,
) -> Result<NegotiationResult, ManifestError> {
    let (manifest, by_id) = parse_capability_manifest(manifest)?;
    let client = parse_version(
        &policy.client_protocol_version,
        "policy.clientProtocolVersion",
    )?;
    let minimum = parse_version(
        &manifest.compatibility.minimum_client_protocol,
        "minimumClientProtocol",
    )?;
    let maximum = parse_version(
        &manifest.compatibility.maximum_client_protocol,
        "maximumClientProtocol",
    )?;

    let mut reasons = Vec::new();
    if compare_versions(&client, &minimum) < 0 || compare_versions(&client, &maximum) > 0 {
        reasons.push("client protocol is outside the server compatibility range".to_string());
    }
    if !policy.allowed_server_builds.is_empty()
        && !policy
            .allowed_server_builds
            .contains(&manifest.server.build_version)
    {
        reasons.push(format!(
            "server build {} is not allowlisted",
            manifest.server.build_version
        ));
    }

    let mut capabilities = HashMap::new();
    for (id, capability) in by_id {
        let effective_status = match capability.status {
            CapabilityStatus::Experimental => {
                let allowed = policy.allow_experimental
                    || policy
                        .allow_experimental_ids
                        .iter()
                        .any(|entry| entry == &id);
                if allowed {
                    CapabilityStatus::Supported
                } else {
                    CapabilityStatus::Unsupported
                }
            }
            ref status => status.clone(),
        };
        capabilities.insert(
            id.clone(),
            NegotiatedCapability {
                declaration: capability,
                effective_status,
            },
        );
    }

    for id in &policy.required_capabilities {
        match capabilities.get(id) {
            Some(capability)
                if matches!(
                    capability.effective_status,
                    CapabilityStatus::Unsupported | CapabilityStatus::Incompatible
                ) =>
            {
                reasons.push(format!("required capability {id} is unavailable"));
            }
            None => reasons.push(format!("required capability {id} is unavailable")),
            _ => {}
        }
    }

    Ok(NegotiationResult {
        status: if reasons.is_empty() {
            "compatible"
        } else {
            "incompatible"
        },
        reasons,
        capabilities,
    })
}

fn validate_status(capability: &CapabilityDeclaration) -> Result<(), ManifestError> {
    match capability.status {
        CapabilityStatus::Unsupported
        | CapabilityStatus::Degraded
        | CapabilityStatus::Incompatible => {
            if capability.reason.is_none() {
                return Err(ManifestError::MissingReason(capability.id.clone()));
            }
        }
        CapabilityStatus::Experimental if !capability.experimental => {
            return Err(ManifestError::MissingReason(
                "experimental flag".to_string(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn parse_version(value: &str, field: &'static str) -> Result<[u32; 3], ManifestError> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return Err(ManifestError::InvalidSemVer(field));
    }
    let mut version = [0_u32; 3];
    for (index, part) in parts.iter().enumerate() {
        version[index] = part
            .parse()
            .map_err(|_| ManifestError::InvalidSemVer(field))?;
    }
    Ok(version)
}

fn compare_versions(left: &[u32; 3], right: &[u32; 3]) -> i32 {
    for index in 0..3 {
        if left[index] != right[index] {
            return if left[index] < right[index] { -1 } else { 1 };
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fixture_manifest_negotiates_compatible() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/codex/fixtures/capability-manifest.v1.json"
        );
        let bytes = fs::read_to_string(fixture_path).expect("read fixture");
        let manifest: CapabilityManifest = serde_json::from_str(&bytes).expect("parse fixture");
        let result = negotiate_capability_manifest(
            manifest,
            &NegotiationPolicy {
                client_protocol_version: "1.0.0".to_string(),
                allowed_server_builds: vec!["fixture-v1".to_string()],
                required_capabilities: vec![
                    "protocol.initialize".to_string(),
                    "thread.lifecycle".to_string(),
                ],
                ..Default::default()
            },
        )
        .expect("negotiate fixture");

        assert_eq!(result.status, "compatible");
        assert_eq!(
            result
                .capabilities
                .get("agents.multi_agent")
                .map(|entry| entry.effective_status.clone()),
            Some(CapabilityStatus::Unsupported)
        );
    }
}
