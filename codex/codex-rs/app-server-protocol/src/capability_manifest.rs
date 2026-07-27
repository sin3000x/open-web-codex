use std::collections::HashMap;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

use crate::ClientNotificationMethod;
use crate::ClientRequestMethod;
use crate::ServerNotificationMethod;
use crate::ServerRequestMethod;
use crate::manifest_method_policy::manifest_method_policy_is_consistent;

/// Top-level capability manifest returned during `initialize`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityManifest {
    pub schema_version: String,
    pub generated_at: String,
    pub server: ServerIdentity,
    pub compatibility: CompatibilityRange,
    #[serde(default)]
    pub capabilities: Vec<CapabilityDeclaration>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServerIdentity {
    pub protocol_version: String,
    pub build_version: String,
    pub commit: String,
    pub target: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityRange {
    pub minimum_client_protocol: String,
    pub maximum_client_protocol: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDeclaration {
    pub id: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub status: CapabilityStatus,
    #[serde(default)]
    pub methods: MethodSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limits: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<StructuredReason>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub experimental: bool,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

impl Default for CapabilityDeclaration {
    fn default() -> Self {
        Self {
            id: String::new(),
            version: default_version(),
            status: CapabilityStatus::Supported,
            methods: MethodSet::default(),
            limits: None,
            reason: None,
            experimental: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
pub enum CapabilityStatus {
    #[serde(rename = "supported")]
    Supported,
    #[serde(rename = "degraded")]
    Degraded,
    #[serde(rename = "unsupported")]
    Unsupported,
    #[serde(rename = "experimental")]
    Experimental,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct MethodSet {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client_requests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_requests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct StructuredReason {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remediation: Option<String>,
}

fn timestamp_now() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let (year, month, day, hour, min, sec) = rfc3339_parts(secs as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn rfc3339_parts(unix_secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = if unix_secs >= 0 {
        unix_secs / 86400
    } else {
        (unix_secs - 86399) / 86400
    };
    let remaining = unix_secs - days * 86400;
    let hour = (remaining / 3600) as u32;
    let min = ((remaining % 3600) / 60) as u32;
    let sec = (remaining % 60) as u32;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32, hour, min, sec)
}

fn apply_registry_derived_metadata(capabilities: &mut [CapabilityDeclaration]) {
    for capability in capabilities {
        capability.experimental = suggested_capability_experimental(capability);
    }
}

pub fn build_manifest() -> CapabilityManifest {
    let mut capabilities = runtime_capabilities();
    apply_registry_derived_metadata(&mut capabilities);
    debug_assert!(
        manifest_methods_are_registered(&capabilities).is_ok(),
        "capability manifest references an unregistered protocol method"
    );
    debug_assert!(
        manifest_experimental_flags_are_consistent(&capabilities).is_ok(),
        "capability manifest experimental flags are inconsistent with the method registry"
    );
    debug_assert!(
        manifest_method_policy_is_consistent(&capabilities).is_ok(),
        "capability manifest method attribution policy is inconsistent"
    );

    CapabilityManifest {
        schema_version: "1.0.0".to_string(),
        generated_at: timestamp_now(),
        server: ServerIdentity {
            protocol_version: "2.0.0".to_string(),
            build_version: env!("CARGO_PKG_VERSION").to_string(),
            commit: option_env!("VERGEN_GIT_SHA")
                .unwrap_or("unknown")
                .to_string(),
            target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        },
        compatibility: CompatibilityRange {
            minimum_client_protocol: "1.0.0".to_string(),
            maximum_client_protocol: "2.0.0".to_string(),
        },
        capabilities,
    }
}

fn manifest_methods_are_registered(capabilities: &[CapabilityDeclaration]) -> Result<(), String> {
    let client_methods = ClientRequestMethod::ALL
        .iter()
        .map(|method| method.wire_name())
        .collect::<std::collections::HashSet<_>>();
    let server_methods = ServerRequestMethod::ALL
        .iter()
        .map(|method| method.wire_name())
        .collect::<std::collections::HashSet<_>>();
    let notification_methods = ServerNotificationMethod::ALL
        .iter()
        .map(|method| method.wire_name())
        .chain(
            ClientNotificationMethod::ALL
                .iter()
                .map(|method| method.wire_name()),
        )
        .collect::<std::collections::HashSet<_>>();

    for capability in capabilities {
        for method in &capability.methods.client_requests {
            if !client_methods.contains(method) {
                return Err(format!(
                    "capability {} references unknown client method {method}",
                    capability.id
                ));
            }
        }
        for method in &capability.methods.server_requests {
            if !server_methods.contains(method) {
                return Err(format!(
                    "capability {} references unknown server method {method}",
                    capability.id
                ));
            }
        }
        for method in &capability.methods.notifications {
            if !notification_methods.contains(method) {
                return Err(format!(
                    "capability {} references unknown notification method {method}",
                    capability.id
                ));
            }
        }
    }
    Ok(())
}

fn client_request_methods(methods: &[ClientRequestMethod]) -> Vec<String> {
    methods.iter().map(|method| method.wire_name()).collect()
}

fn server_request_methods(methods: &[ServerRequestMethod]) -> Vec<String> {
    methods.iter().map(|method| method.wire_name()).collect()
}

fn server_notification_methods(methods: &[ServerNotificationMethod]) -> Vec<String> {
    methods.iter().map(|method| method.wire_name()).collect()
}

fn client_notification_methods(methods: &[ClientNotificationMethod]) -> Vec<String> {
    methods.iter().map(|method| method.wire_name()).collect()
}

/// Returns every wire method marked experimental in the protocol registry.
pub fn registry_experimental_wire_methods() -> Vec<String> {
    let mut methods = ClientRequestMethod::ALL
        .iter()
        .filter_map(|method| method.experimental_reason().map(|_| method.wire_name()))
        .chain(
            ServerRequestMethod::ALL
                .iter()
                .filter_map(|method| method.experimental_reason().map(|_| method.wire_name())),
        )
        .chain(
            ServerNotificationMethod::ALL
                .iter()
                .filter_map(|method| method.experimental_reason().map(|_| method.wire_name())),
        )
        .chain(
            ClientNotificationMethod::ALL
                .iter()
                .filter_map(|method| method.experimental_reason().map(|_| method.wire_name())),
        )
        .collect::<Vec<_>>();
    methods.sort();
    methods.dedup();
    methods
}

/// Suggested `experimental` flag derived purely from registry annotations.
pub fn suggested_capability_experimental(capability: &CapabilityDeclaration) -> bool {
    capability_references_experimental_method(capability)
        || PRODUCT_EXPERIMENTAL_CAPABILITY_IDS.contains(&capability.id.as_str())
}

/// Returns the experimental reason for a registered wire method, if any.
pub fn registry_method_experimental_reason(method: &str) -> Option<&'static str> {
    ClientRequestMethod::ALL
        .iter()
        .find(|entry| entry.wire_name() == method)
        .and_then(|entry| entry.experimental_reason())
        .or_else(|| {
            ServerRequestMethod::ALL
                .iter()
                .find(|entry| entry.wire_name() == method)
                .and_then(|entry| entry.experimental_reason())
        })
        .or_else(|| {
            ServerNotificationMethod::ALL
                .iter()
                .find(|entry| entry.wire_name() == method)
                .and_then(|entry| entry.experimental_reason())
        })
        .or_else(|| {
            ClientNotificationMethod::ALL
                .iter()
                .find(|entry| entry.wire_name() == method)
                .and_then(|entry| entry.experimental_reason())
        })
}

fn capability_references_experimental_method(capability: &CapabilityDeclaration) -> bool {
    capability
        .methods
        .client_requests
        .iter()
        .chain(capability.methods.server_requests.iter())
        .chain(capability.methods.notifications.iter())
        .any(|method| registry_method_experimental_reason(method).is_some())
}

/// Capabilities that declare product-level experimental status without relying
/// solely on protocol `#[experimental]` method annotations.
const PRODUCT_EXPERIMENTAL_CAPABILITY_IDS: &[&str] = &["agents.multi_agent"];

fn manifest_experimental_flags_are_consistent(
    capabilities: &[CapabilityDeclaration],
) -> Result<(), String> {
    for capability in capabilities {
        let references_experimental = capability_references_experimental_method(capability);
        let product_experimental =
            PRODUCT_EXPERIMENTAL_CAPABILITY_IDS.contains(&capability.id.as_str());

        if references_experimental && !capability.experimental {
            return Err(format!(
                "capability {} references experimental protocol methods but experimental=false",
                capability.id
            ));
        }

        if capability.experimental && !references_experimental && !product_experimental {
            return Err(format!(
                "capability {} is marked experimental without experimental methods or an allowlisted product reason",
                capability.id
            ));
        }

        if capability.status == CapabilityStatus::Experimental && !capability.experimental {
            return Err(format!(
                "capability {} has Experimental status but experimental=false",
                capability.id
            ));
        }
    }
    Ok(())
}

fn runtime_capabilities() -> Vec<CapabilityDeclaration> {
    vec![
        CapabilityDeclaration {
            id: "protocol.initialize".into(),
            status: CapabilityStatus::Supported,
            methods: MethodSet {
                client_requests: client_request_methods(&[ClientRequestMethod::Initialize]),
                notifications: client_notification_methods(&[
                    ClientNotificationMethod::Initialized,
                ]),
                ..Default::default()
            },
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "thread.lifecycle".into(),
            status: CapabilityStatus::Supported,
            methods: MethodSet {
                client_requests: client_request_methods(&[
                    ClientRequestMethod::ThreadStart,
                    ClientRequestMethod::ThreadRead,
                    ClientRequestMethod::ThreadResume,
                    ClientRequestMethod::ThreadList,
                    ClientRequestMethod::ThreadArchive,
                ]),
                notifications: server_notification_methods(&[
                    ServerNotificationMethod::ThreadStarted,
                    ServerNotificationMethod::ThreadStatusChanged,
                    ServerNotificationMethod::ThreadArchived,
                ]),
                ..Default::default()
            },
            limits: Some(HashMap::from([(
                "maxConcurrentThreads".into(),
                serde_json::json!(16),
            )])),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "turn.lifecycle".into(),
            status: CapabilityStatus::Supported,
            methods: MethodSet {
                client_requests: client_request_methods(&[
                    ClientRequestMethod::TurnStart,
                    ClientRequestMethod::TurnSteer,
                    ClientRequestMethod::TurnInterrupt,
                ]),
                notifications: server_notification_methods(&[
                    ServerNotificationMethod::TurnStarted,
                    ServerNotificationMethod::TurnCompleted,
                ]),
                ..Default::default()
            },
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "approval.lifecycle".into(),
            status: CapabilityStatus::Supported,
            methods: MethodSet {
                server_requests: server_request_methods(&[
                    ServerRequestMethod::CommandExecutionRequestApproval,
                    ServerRequestMethod::FileChangeRequestApproval,
                    ServerRequestMethod::PermissionsRequestApproval,
                    ServerRequestMethod::ToolRequestUserInput,
                ]),
                ..Default::default()
            },
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "profile.multi_workspace".into(),
            status: CapabilityStatus::Supported,
            methods: MethodSet {
                client_requests: client_request_methods(&[
                    ClientRequestMethod::ThreadStart,
                    ClientRequestMethod::ThreadList,
                ]),
                ..Default::default()
            },
            limits: Some(HashMap::from([(
                "maxWorkspacesPerProfile".into(),
                serde_json::json!(8),
            )])),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "memory.lifecycle".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "memory.bridge.missing".into(),
                message: "Memory status, export and reset are unavailable.".into(),
                remediation: Some("Install a build that implements CR-104 through CR-108.".into()),
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "agents.crud".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "agents.bridge.missing".into(),
                message: "Native Agent CRUD is unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "agents.multi_agent".into(),
            status: CapabilityStatus::Experimental,
            methods: MethodSet {
                notifications: server_notification_methods(&[
                    ServerNotificationMethod::ItemStarted,
                    ServerNotificationMethod::ItemCompleted,
                    ServerNotificationMethod::ThreadStarted,
                ]),
                ..Default::default()
            },
            limits: Some(HashMap::from([
                ("maxAgentThreads".into(), serde_json::json!(8)),
                ("maxAgentDepth".into(), serde_json::json!(3)),
            ])),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "skills.crud".into(),
            status: CapabilityStatus::Degraded,
            methods: MethodSet {
                client_requests: client_request_methods(&[ClientRequestMethod::SkillsList]),
                ..Default::default()
            },
            reason: Some(StructuredReason {
                code: "skills.write.missing".into(),
                message: "Skill listing is available but write operations are not.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "skills.validation".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "skills.validation.missing".into(),
                message: "Native Skill validation is unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "skills.test".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "skills.test.missing".into(),
                message: "The isolated Skill test hook is unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "plugins.lifecycle".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "plugins.bridge.missing".into(),
                message: "Plugin lifecycle operations are unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "plugins.permissions".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "plugins.permissions.missing".into(),
                message: "Plugin permission metadata is unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "mcp.config".into(),
            status: CapabilityStatus::Degraded,
            methods: MethodSet {
                client_requests: client_request_methods(&[
                    ClientRequestMethod::McpServerStatusList,
                ]),
                ..Default::default()
            },
            limits: Some(HashMap::from([(
                "maxMcpServers".into(),
                serde_json::json!(16),
            )])),
            reason: Some(StructuredReason {
                code: "mcp.config.read_only".into(),
                message: "MCP status is available but configuration and reload are not.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "mcp.oauth".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "mcp.oauth.missing".into(),
                message: "MCP OAuth is unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "mcp.elicitation".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "mcp.elicitation.missing".into(),
                message: "MCP elicitation is unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "tools.discovery".into(),
            status: CapabilityStatus::Unsupported,
            reason: Some(StructuredReason {
                code: "tools.discovery.missing".into(),
                message: "Managed tool discovery metadata is unavailable.".into(),
                remediation: None,
            }),
            ..Default::default()
        },
        CapabilityDeclaration {
            id: "models.providers".into(),
            status: CapabilityStatus::Supported,
            methods: MethodSet {
                client_requests: client_request_methods(&[
                    ClientRequestMethod::ModelProviderList,
                    ClientRequestMethod::ModelList,
                    ClientRequestMethod::ConfigBatchWrite,
                ]),
                ..Default::default()
            },
            ..Default::default()
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_valid_json() {
        let manifest = build_manifest();
        let json = serde_json::to_value(&manifest).expect("serialize manifest");
        assert_eq!(json["schemaVersion"], "1.0.0");
        assert!(json["generatedAt"].is_string());
        assert_eq!(json["server"]["protocolVersion"], "2.0.0");
        assert!(json["capabilities"].is_array());
        assert!(!json["capabilities"].as_array().unwrap().is_empty());
    }

    #[test]
    fn manifest_has_runtime_capabilities() {
        let manifest = build_manifest();
        let ids: Vec<&str> = manifest
            .capabilities
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert!(ids.contains(&"protocol.initialize"));
        assert!(ids.contains(&"thread.lifecycle"));
        assert!(ids.contains(&"turn.lifecycle"));
        assert!(ids.contains(&"approval.lifecycle"));
    }

    #[test]
    fn supported_capabilities_have_methods() {
        let manifest = build_manifest();
        for cap in &manifest.capabilities {
            match cap.status {
                CapabilityStatus::Supported => {
                    assert!(
                        !cap.methods.client_requests.is_empty()
                            || !cap.methods.server_requests.is_empty()
                            || !cap.methods.notifications.is_empty(),
                        "Supported capability '{}' must have at least one method",
                        cap.id
                    );
                }
                CapabilityStatus::Unsupported => {
                    assert!(
                        cap.reason.is_some(),
                        "Unsupported capability '{}' should provide a reason",
                        cap.id
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn timestamp_is_valid() {
        let ts = timestamp_now();
        assert_eq!(ts.len(), 20, "expected 20-char ISO 8601, got {ts}");
        assert!(ts.ends_with('Z'));
    }

    #[test]
    fn manifest_roundtrip() {
        let manifest = build_manifest();
        let json = serde_json::to_value(&manifest).expect("serialize");
        let back: CapabilityManifest = serde_json::from_value(json).expect("deserialize");
        assert_eq!(manifest, back);
    }

    #[test]
    fn manifest_request_methods_are_registered() {
        let manifest = build_manifest();
        manifest_methods_are_registered(&manifest.capabilities)
            .expect("manifest request methods must come from the protocol registry");
    }

    #[test]
    fn manifest_experimental_flags_match_registry() {
        let manifest = build_manifest();
        manifest_experimental_flags_are_consistent(&manifest.capabilities)
            .expect("manifest experimental flags must match registry annotations");
    }

    #[test]
    fn registry_exposes_experimental_notification_methods() {
        assert_eq!(
            registry_method_experimental_reason("thread/settings/updated"),
            Some("thread/settings/updated")
        );
        assert_eq!(registry_method_experimental_reason("initialized"), None);
        assert!(
            ServerNotificationMethod::ALL
                .iter()
                .any(|method| method.wire_name() == "item/started")
        );
        assert!(
            ClientNotificationMethod::ALL
                .iter()
                .any(|method| method.wire_name() == "initialized")
        );
    }

    #[test]
    fn manifest_method_policy_is_consistent_for_runtime() {
        let manifest = build_manifest();
        crate::manifest_method_policy::manifest_method_policy_is_consistent(&manifest.capabilities)
            .expect("runtime manifest must satisfy method attribution policy");
    }

    #[test]
    fn suggested_experimental_flags_match_declared_runtime_capabilities() {
        let manifest = build_manifest();
        for capability in &manifest.capabilities {
            assert_eq!(
                capability.experimental,
                suggested_capability_experimental(capability),
                "capability {} experimental flag should match registry-derived suggestion",
                capability.id
            );
        }
    }

    #[test]
    fn registry_lists_experimental_wire_methods() {
        let methods = registry_experimental_wire_methods();
        assert!(
            methods.contains(&"thread/settings/update".to_string()),
            "expected experimental client request in registry listing"
        );
        assert!(
            methods.contains(&"thread/settings/updated".to_string()),
            "expected experimental notification in registry listing"
        );
    }

    #[test]
    fn runtime_manifest_method_coverage_is_tracked() {
        let manifest = build_manifest();
        let attributed = manifest
            .capabilities
            .iter()
            .flat_map(|capability| {
                capability
                    .methods
                    .client_requests
                    .iter()
                    .chain(capability.methods.server_requests.iter())
                    .chain(capability.methods.notifications.iter())
                    .cloned()
            })
            .collect::<std::collections::HashSet<_>>();

        let registered = ClientRequestMethod::ALL
            .iter()
            .map(|method| method.wire_name())
            .chain(
                ServerRequestMethod::ALL
                    .iter()
                    .map(|method| method.wire_name()),
            )
            .chain(
                ServerNotificationMethod::ALL
                    .iter()
                    .map(|method| method.wire_name()),
            )
            .chain(
                ClientNotificationMethod::ALL
                    .iter()
                    .map(|method| method.wire_name()),
            )
            .collect::<std::collections::HashSet<_>>();

        assert!(
            attributed.is_subset(&registered),
            "manifest methods must be a subset of the protocol registry"
        );
        // The Runtime manifest intentionally declares a focused product subset.
        assert!(
            !attributed.is_empty(),
            "runtime manifest must attribute at least one method"
        );
        assert!(
            registered.len() > attributed.len(),
            "expected unattributed registry methods while full coverage policy is incomplete"
        );
    }
}
