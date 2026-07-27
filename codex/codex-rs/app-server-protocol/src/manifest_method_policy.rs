use std::collections::HashSet;

use crate::ClientNotificationMethod;
use crate::ClientRequestMethod;
use crate::ServerNotificationMethod;
use crate::ServerRequestMethod;
use crate::capability_manifest::CapabilityDeclaration;

/// Wire methods intentionally excluded from product capability attribution.
///
/// These methods may exist in the protocol registry but are internal,
/// deprecated, or not part of the Web platform contract. Exclusion here does
/// not remove Runtime support; it only means Alpha Manifest does not promise
/// them to browsers.
pub const EXCLUDED_FROM_PRODUCT_ATTRIBUTION: &[ServerNotificationMethod] = &[
    ServerNotificationMethod::RawResponseItemCompleted,
    ServerNotificationMethod::RawResponseCompleted,
    ServerNotificationMethod::ContextCompacted,
];

pub fn all_registered_wire_methods() -> HashSet<String> {
    ClientRequestMethod::ALL
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
        .collect()
}

pub fn attributed_manifest_methods(capabilities: &[CapabilityDeclaration]) -> HashSet<String> {
    capabilities
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
        .collect()
}

pub fn excluded_from_product_attribution() -> HashSet<String> {
    EXCLUDED_FROM_PRODUCT_ATTRIBUTION
        .iter()
        .map(|method| method.wire_name())
        .collect()
}

pub fn manifest_method_policy_is_consistent(
    capabilities: &[CapabilityDeclaration],
) -> Result<(), String> {
    let registered = all_registered_wire_methods();
    let attributed = attributed_manifest_methods(capabilities);
    let excluded = excluded_from_product_attribution();

    if !attributed.is_subset(&registered) {
        return Err("manifest attributed methods must be a subset of the protocol registry".into());
    }

    for method in &attributed {
        if excluded.contains(method) {
            return Err(format!(
                "capability attribution must not include excluded method {method}"
            ));
        }
    }

    for method in EXCLUDED_FROM_PRODUCT_ATTRIBUTION {
        let method = method.wire_name();
        if !registered.contains(&method) {
            return Err(format!(
                "excluded method {method} is not registered in the protocol"
            ));
        }
    }

    Ok(())
}

/// Returns unattributed registry methods that are neither promised nor excluded.
pub fn unattributed_registry_methods(capabilities: &[CapabilityDeclaration]) -> Vec<String> {
    let registered = all_registered_wire_methods();
    let attributed = attributed_manifest_methods(capabilities);
    let excluded = excluded_from_product_attribution();

    let mut unattributed = registered
        .into_iter()
        .filter(|method| !attributed.contains(method) && !excluded.contains(method))
        .collect::<Vec<_>>();
    unattributed.sort();
    unattributed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_manifest;

    #[test]
    fn excluded_methods_are_registered_and_not_attributed() {
        let manifest = build_manifest();
        manifest_method_policy_is_consistent(&manifest.capabilities)
            .expect("manifest method policy must be consistent");

        let attributed = attributed_manifest_methods(&manifest.capabilities);
        for method in EXCLUDED_FROM_PRODUCT_ATTRIBUTION {
            let method = method.wire_name();
            assert!(
                !attributed.contains(&method),
                "excluded method {method} must not appear in capability attribution"
            );
        }
    }

    #[test]
    fn runtime_manifest_leaves_internal_methods_unattributed() {
        let manifest = build_manifest();
        let unattributed = unattributed_registry_methods(&manifest.capabilities);
        assert!(
            !unattributed.is_empty(),
            "runtime manifest should not attribute every registry method"
        );
        assert!(
            unattributed.contains(&"thread/deleted".to_string()),
            "expected unattributed lifecycle methods while product coverage is partial"
        );
    }
}
