use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use open_web_codex_platform_contracts::{
    AvailableCopilotPackage, CopilotInstallationState, CopilotInstallationSummary,
    CopilotProfileStatus,
};
use open_web_codex_profile_host::{ProfileHost, ProfileStartupFile, ProfileStartupFileRemoval};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use thiserror::Error;

use crate::copilot_package::{CopilotPackageAssets, CopilotPackageError};
use crate::delivery_contracts::DeliveryRegistry;

const MARK_FAILURE_SQL: &str = "UPDATE profile_copilot_installations SET \
         last_failure_kind = $3, last_failure_code = $4, updated_at = now() \
     WHERE profile_id = (SELECT id FROM profiles WHERE runtime_key = $1) \
       AND package_id = $2";
const MARK_UNAVAILABLE_CLEANED_SQL: &str =
    "UPDATE profile_copilot_installations SET configured_revision = NULL, \
         managed_skill_ids = '{}', managed_agent_role_ids = '{}', \
         last_failure_kind = 'unavailable', last_failure_code = $3, updated_at = now() \
     WHERE profile_id = (SELECT id FROM profiles WHERE runtime_key = $1) \
       AND package_id = $2";

#[derive(Debug, Clone)]
pub(crate) struct CopilotPackageSource {
    pub id: String,
    pub package_root: PathBuf,
    pub prepared_descriptor: PathBuf,
    pub build_store_root: PathBuf,
}

#[derive(Clone, Default)]
pub(crate) struct CopilotSourceRegistry {
    sources: Arc<BTreeMap<String, RegisteredSource>>,
}

#[derive(Clone)]
enum RegisteredSource {
    Available(Arc<CopilotPackageAssets>),
    Unavailable,
}

impl CopilotSourceRegistry {
    pub(crate) fn discover(
        packages_root: &std::path::Path,
        prepared_root: &std::path::Path,
        build_store_root: &std::path::Path,
    ) -> Result<Self, CopilotInstallationError> {
        let packages_root = packages_root.canonicalize().map_err(|error| {
            CopilotInstallationError::InvalidSource(format!(
                "Copilot packages directory is unavailable: {error}"
            ))
        })?;
        let prepared_root = prepared_root.canonicalize().map_err(|error| {
            CopilotInstallationError::InvalidSource(format!(
                "Copilot prepared directory is unavailable: {error}"
            ))
        })?;
        let build_store_root = build_store_root.canonicalize().map_err(|error| {
            CopilotInstallationError::InvalidSource(format!(
                "Copilot build store directory is unavailable: {error}"
            ))
        })?;
        let mut roots = std::fs::read_dir(&packages_root)
            .map_err(|error| CopilotInstallationError::InvalidSource(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CopilotInstallationError::InvalidSource(error.to_string()))?;
        roots.sort_by_key(std::fs::DirEntry::file_name);
        let mut sources = Vec::new();
        for entry in roots {
            let file_type = entry
                .file_type()
                .map_err(|error| CopilotInstallationError::InvalidSource(error.to_string()))?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let package_root = entry.path();
            if !package_root.join("copilot.toml").is_file() {
                continue;
            }
            let id = CopilotPackageAssets::manifest_id(&package_root)?;
            sources.push(CopilotPackageSource {
                id: id.clone(),
                package_root,
                prepared_descriptor: prepared_root
                    .join(id)
                    .join("copilot-sdk/prepared-tools.v1.json"),
                build_store_root: build_store_root.clone(),
            });
        }
        Self::load(sources)
    }

    pub(crate) fn load(
        sources: Vec<CopilotPackageSource>,
    ) -> Result<Self, CopilotInstallationError> {
        let mut registered = BTreeMap::new();
        for source in sources {
            validate_id(&source.id)?;
            if registered.contains_key(&source.id) {
                return Err(CopilotInstallationError::InvalidSource(format!(
                    "duplicate application Copilot source id {}",
                    source.id
                )));
            }
            let loaded = match CopilotPackageAssets::resolve(
                &source.package_root,
                &source.prepared_descriptor,
                &source.build_store_root,
            ) {
                Ok(assets) if assets.id() == source.id => {
                    RegisteredSource::Available(Arc::new(assets))
                }
                Ok(assets) => {
                    return Err(CopilotInstallationError::InvalidSource(format!(
                        "application source id {} does not match package id {}",
                        source.id,
                        assets.id()
                    )))
                }
                Err(error) => {
                    tracing::warn!(package_id = %source.id, error = %error, "application Copilot source is unavailable");
                    RegisteredSource::Unavailable
                }
            };
            registered.insert(source.id, loaded);
        }
        Ok(Self {
            sources: Arc::new(registered),
        })
    }

    pub(crate) fn available(&self, id: &str) -> Option<Arc<CopilotPackageAssets>> {
        match self.sources.get(id) {
            Some(RegisteredSource::Available(assets)) => Some(assets.clone()),
            _ => None,
        }
    }

    pub(crate) fn available_assets(&self) -> Vec<Arc<CopilotPackageAssets>> {
        self.sources
            .values()
            .filter_map(|source| match source {
                RegisteredSource::Available(assets) => Some(assets.clone()),
                RegisteredSource::Unavailable => None,
            })
            .collect()
    }

    fn contains(&self, id: &str) -> bool {
        self.sources.contains_key(id)
    }

    fn summaries(&self) -> Vec<AvailableCopilotPackage> {
        self.sources
            .iter()
            .map(|(id, source)| match source {
                RegisteredSource::Available(assets) => AvailableCopilotPackage {
                    package_id: id.clone(),
                    available: true,
                    display_name: Some(assets.display_name().to_string()),
                },
                RegisteredSource::Unavailable => AvailableCopilotPackage {
                    package_id: id.clone(),
                    available: false,
                    display_name: None,
                },
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskCopilotSelection {
    pub package_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct InstallationRecord {
    package_id: String,
    desired_active: bool,
    source_revision: String,
    configured_revision: Option<String>,
    managed_skill_ids: Vec<String>,
    managed_agent_role_ids: Vec<String>,
    last_failure_kind: Option<String>,
    last_failure_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeactivationPlan {
    preserve_unavailable: bool,
}

fn plan_deactivation(
    record: Option<&InstallationRecord>,
    source_available: bool,
) -> Result<DeactivationPlan, CopilotInstallationError> {
    record.ok_or(CopilotInstallationError::NotFound)?;
    Ok(DeactivationPlan {
        preserve_unavailable: !source_available,
    })
}

#[derive(Clone)]
pub(crate) struct CopilotInstallationStore {
    db: PgPool,
    runtime_key: String,
}

impl CopilotInstallationStore {
    pub(crate) fn new(db: PgPool, runtime_key: impl Into<String>) -> Self {
        Self {
            db,
            runtime_key: runtime_key.into(),
        }
    }

    pub(crate) async fn load_all(&self) -> Result<Vec<InstallationRecord>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT installation.package_id, installation.desired_active, \
                    installation.source_revision, installation.configured_revision, \
                    installation.managed_skill_ids, installation.managed_agent_role_ids, \
                    installation.last_failure_kind, installation.last_failure_code \
             FROM profile_copilot_installations installation \
             JOIN profiles profile ON profile.id = installation.profile_id \
             WHERE profile.runtime_key = $1
             ORDER BY installation.package_id",
        )
        .bind(&self.runtime_key)
        .fetch_all(&self.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| InstallationRecord {
                package_id: row.get("package_id"),
                desired_active: row.get("desired_active"),
                source_revision: row.get("source_revision"),
                configured_revision: row.get("configured_revision"),
                managed_skill_ids: row.get("managed_skill_ids"),
                managed_agent_role_ids: row.get("managed_agent_role_ids"),
                last_failure_kind: row.get("last_failure_kind"),
                last_failure_code: row.get("last_failure_code"),
            })
            .collect())
    }

    pub(crate) async fn load_package(
        &self,
        package_id: &str,
    ) -> Result<Option<InstallationRecord>, sqlx::Error> {
        Ok(self
            .load_all()
            .await?
            .into_iter()
            .find(|record| record.package_id == package_id))
    }

    pub(crate) async fn ensure_available(
        &self,
        assets: &CopilotPackageAssets,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO profile_copilot_installations \
                 (profile_id, package_id, desired_active, source_revision) \
             SELECT id, $2, TRUE, $3 FROM profiles WHERE runtime_key = $1 \
             ON CONFLICT (profile_id, package_id) DO NOTHING",
        )
        .bind(&self.runtime_key)
        .bind(assets.id())
        .bind(assets.source_revision())
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn activate(&self, assets: &CopilotPackageAssets) -> Result<(), sqlx::Error> {
        self.activate_values(assets.id(), assets.source_revision())
            .await
    }

    async fn activate_values(
        &self,
        package_id: &str,
        source_revision: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO profile_copilot_installations \
                 (profile_id, package_id, desired_active, source_revision) \
             SELECT id, $2, TRUE, $3 FROM profiles WHERE runtime_key = $1 \
             ON CONFLICT (profile_id, package_id) DO UPDATE SET \
                 desired_active = TRUE, \
                 source_revision = EXCLUDED.source_revision, \
                 last_failure_kind = NULL, last_failure_code = NULL, updated_at = now()",
        )
        .bind(&self.runtime_key)
        .bind(package_id)
        .bind(source_revision)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn deactivate(&self, package_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE profile_copilot_installations SET desired_active = FALSE, \
                 last_failure_kind = NULL, last_failure_code = NULL, updated_at = now() \
             WHERE profile_id = (SELECT id FROM profiles WHERE runtime_key = $1) \
               AND package_id = $2",
        )
        .bind(&self.runtime_key)
        .bind(package_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn mark_configured(
        &self,
        package_id: &str,
        revision: Option<&str>,
        skill_ids: &[String],
        role_ids: &[String],
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE profile_copilot_installations SET configured_revision = $3, \
                 managed_skill_ids = $4, managed_agent_role_ids = $5, \
                 last_failure_kind = NULL, last_failure_code = NULL, updated_at = now() \
             WHERE profile_id = (SELECT id FROM profiles WHERE runtime_key = $1) \
               AND package_id = $2",
        )
        .bind(&self.runtime_key)
        .bind(package_id)
        .bind(revision)
        .bind(skill_ids)
        .bind(role_ids)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn mark_unavailable_cleaned(
        &self,
        package_id: &str,
        code: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(MARK_UNAVAILABLE_CLEANED_SQL)
            .bind(&self.runtime_key)
            .bind(package_id)
            .bind(code)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// The current local/private contract follows the application's registered
    /// source on cold restart. Preserve the last configured revision and owned
    /// destinations until the new source has reconciled successfully.
    pub(crate) async fn refresh_source_revision(
        &self,
        package_id: &str,
        revision: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE profile_copilot_installations SET source_revision = $3, \
                 last_failure_kind = NULL, last_failure_code = NULL, updated_at = now() \
             WHERE profile_id = (SELECT id FROM profiles WHERE runtime_key = $1) \
               AND package_id = $2 AND desired_active",
        )
        .bind(&self.runtime_key)
        .bind(package_id)
        .bind(revision)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub(crate) async fn mark_failure(
        &self,
        package_id: &str,
        kind: &str,
        code: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(MARK_FAILURE_SQL)
            .bind(&self.runtime_key)
            .bind(package_id)
            .bind(kind)
            .bind(code)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

pub(crate) struct ColdStartComposition {
    pub startup_files: Vec<ProfileStartupFile>,
    pub removed_startup_files: Vec<ProfileStartupFileRemoval>,
    pub root_execution_configs: Vec<open_web_codex_adapter::real::RootExecutionConfig>,
    pub deliveries: DeliveryRegistry,
    pub active_package_ids: Vec<String>,
    pub completion: ColdStartCompletion,
}

/// Persisted installation state is advanced only after ProfileHost has
/// reconciled the native files, initialized app-server, and registered the
/// running Profile. A failed Host start keeps the last configured revision and
/// managed destinations recoverable for the next cold start.
pub(crate) struct ColdStartCompletion {
    changes: Vec<ColdStartChange>,
}

enum ColdStartChange {
    Configure {
        package_id: String,
        revision: String,
        skill_ids: Vec<String>,
        role_ids: Vec<String>,
    },
    Deactivate {
        package_id: String,
        preserve_unavailable: bool,
    },
    CleanUnavailable {
        package_id: String,
    },
}

impl ColdStartCompletion {
    pub(crate) async fn mark_host_ready(
        &self,
        store: &CopilotInstallationStore,
    ) -> Result<(), sqlx::Error> {
        for change in &self.changes {
            match change {
                ColdStartChange::Configure {
                    package_id,
                    revision,
                    skill_ids,
                    role_ids,
                } => {
                    store
                        .mark_configured(package_id, Some(revision), skill_ids, role_ids)
                        .await?;
                }
                ColdStartChange::Deactivate {
                    package_id,
                    preserve_unavailable,
                } => {
                    if *preserve_unavailable {
                        store
                            .mark_unavailable_cleaned(package_id, "application_source_unavailable")
                            .await?;
                    } else {
                        store.mark_configured(package_id, None, &[], &[]).await?;
                    }
                }
                ColdStartChange::CleanUnavailable { package_id } => {
                    store
                        .mark_unavailable_cleaned(package_id, "application_source_unavailable")
                        .await?;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn mark_host_failed(
        &self,
        store: &CopilotInstallationStore,
        code: &str,
    ) -> Result<(), sqlx::Error> {
        for change in &self.changes {
            match change {
                ColdStartChange::CleanUnavailable { package_id }
                | ColdStartChange::Deactivate {
                    package_id,
                    preserve_unavailable: true,
                } => {
                    store
                        .mark_failure(package_id, "unavailable", "application_source_unavailable")
                        .await?;
                }
                ColdStartChange::Configure { package_id, .. }
                | ColdStartChange::Deactivate {
                    package_id,
                    preserve_unavailable: false,
                } => {
                    store.mark_failure(package_id, "failed", code).await?;
                }
            }
        }
        Ok(())
    }
}

pub(crate) async fn cold_start_composition(
    store: &CopilotInstallationStore,
    sources: &CopilotSourceRegistry,
    profile_home: &std::path::Path,
) -> Result<ColdStartComposition, CopilotInstallationError> {
    let records = store.load_all().await?;
    let mut startup_files = Vec::new();
    let mut root_execution_configs = Vec::new();
    let mut delivery_registries = Vec::new();
    let mut active_package_ids = Vec::new();
    let mut keep_skills = BTreeSet::new();
    let mut keep_roles = BTreeSet::new();
    let mut changes = Vec::new();

    for record in &records {
        if record.desired_active {
            let Some(assets) = sources.available(&record.package_id) else {
                store
                    .mark_failure(
                        &record.package_id,
                        "unavailable",
                        "application_source_unavailable",
                    )
                    .await?;
                changes.push(ColdStartChange::CleanUnavailable {
                    package_id: record.package_id.clone(),
                });
                continue;
            };
            if source_revision_requires_refresh(&record.source_revision, assets.source_revision()) {
                store
                    .refresh_source_revision(&record.package_id, assets.source_revision())
                    .await?;
            }
            let skill_ids = assets.skill_ids();
            let role_ids = assets.agent_role_ids();
            if skill_ids.iter().any(|id| keep_skills.contains(id))
                || role_ids.iter().any(|id| keep_roles.contains(id))
            {
                store
                    .mark_failure(&record.package_id, "failed", "profile_destination_conflict")
                    .await?;
                return Err(CopilotInstallationError::InvalidSource(format!(
                    "Copilot package {} conflicts with another active Profile destination",
                    record.package_id
                )));
            }
            let package_files = match assets.startup_files(profile_home) {
                Ok(files) => files,
                Err(error) => {
                    store
                        .mark_failure(&record.package_id, "failed", "profile_composition_failed")
                        .await?;
                    return Err(error.into());
                }
            };
            startup_files.extend(package_files);
            root_execution_configs.push(assets.root_execution_config(profile_home)?);
            delivery_registries.push(assets.deliveries());
            keep_skills.extend(skill_ids.iter().cloned());
            keep_roles.extend(role_ids.iter().cloned());
            changes.push(ColdStartChange::Configure {
                package_id: record.package_id.clone(),
                revision: assets.source_revision().to_string(),
                skill_ids,
                role_ids,
            });
            active_package_ids.push(record.package_id.clone());
        } else {
            changes.push(ColdStartChange::Deactivate {
                package_id: record.package_id.clone(),
                preserve_unavailable: sources.available(&record.package_id).is_none(),
            });
        }
    }
    let mut removed_startup_files = Vec::new();
    for record in &records {
        removed_startup_files.extend(removals_for(record, &keep_skills, &keep_roles)?);
    }
    let deliveries = DeliveryRegistry::merge(delivery_registries)
        .map_err(CopilotInstallationError::InvalidSource)?;
    Ok(ColdStartComposition {
        startup_files,
        removed_startup_files,
        root_execution_configs,
        deliveries,
        active_package_ids,
        completion: ColdStartCompletion { changes },
    })
}

fn removals_for(
    record: &InstallationRecord,
    keep_skills: &BTreeSet<String>,
    keep_roles: &BTreeSet<String>,
) -> Result<Vec<ProfileStartupFileRemoval>, CopilotInstallationError> {
    let mut removals = Vec::new();
    for id in &record.managed_skill_ids {
        if !keep_skills.contains(id) {
            removals.push(ProfileStartupFileRemoval::package_skill(id.clone())?);
        }
    }
    for id in &record.managed_agent_role_ids {
        if !keep_roles.contains(id) {
            removals.push(ProfileStartupFileRemoval::package_agent_role(id.clone())?);
        }
    }
    Ok(removals)
}

#[derive(Clone)]
pub(crate) struct CopilotInstallationService {
    store: CopilotInstallationStore,
    sources: CopilotSourceRegistry,
    runtime: Option<ProfileHost>,
    runtime_workspace: Option<PathBuf>,
    operation: Arc<tokio::sync::Mutex<()>>,
}

impl CopilotInstallationService {
    pub(crate) fn new(
        store: CopilotInstallationStore,
        sources: CopilotSourceRegistry,
        runtime: Option<ProfileHost>,
        runtime_workspace: Option<PathBuf>,
    ) -> Self {
        Self {
            store,
            sources,
            runtime,
            runtime_workspace,
            operation: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) async fn status(&self) -> Result<CopilotProfileStatus, CopilotInstallationError> {
        let discovery = self.discover_skills().await;
        let mut installations = Vec::new();
        for record in self.store.load_all().await? {
            installations.push(self.summary(record, discovery.as_ref()));
        }
        Ok(CopilotProfileStatus {
            packages: self.sources.summaries(),
            installations,
        })
    }

    pub(crate) async fn task_selection(
        &self,
        requested_package_id: Option<&str>,
    ) -> Result<Option<TaskCopilotSelection>, CopilotInstallationError> {
        let Some(package_id) = requested_package_id else {
            return if self.sources.available_assets().is_empty() {
                Ok(None)
            } else {
                Err(CopilotInstallationError::InvalidSelection)
            };
        };
        validate_id(package_id)?;
        let Some(record) = self.store.load_package(package_id).await? else {
            return Err(CopilotInstallationError::Unavailable);
        };
        if !record.desired_active {
            return Err(CopilotInstallationError::Unavailable);
        }
        let assets = self
            .sources
            .available(package_id)
            .ok_or(CopilotInstallationError::Unavailable)?;
        if record.configured_revision.as_deref() != Some(assets.source_revision()) {
            return Err(CopilotInstallationError::Unavailable);
        }
        Ok(Some(TaskCopilotSelection {
            package_id: package_id.to_string(),
        }))
    }

    pub(crate) async fn activate(
        &self,
        package_id: &str,
    ) -> Result<CopilotProfileStatus, CopilotInstallationError> {
        let _operation = self.operation.lock().await;
        validate_id(package_id)?;
        let assets = self.sources.available(package_id).ok_or_else(|| {
            if self.sources.contains(package_id) {
                CopilotInstallationError::Unavailable
            } else {
                CopilotInstallationError::NotFound
            }
        })?;
        self.store.activate(&assets).await?;
        self.status().await
    }

    pub(crate) async fn deactivate(
        &self,
        package_id: &str,
    ) -> Result<CopilotProfileStatus, CopilotInstallationError> {
        let _operation = self.operation.lock().await;
        validate_id(package_id)?;
        let record = self.store.load_package(package_id).await?;
        let plan = plan_deactivation(
            record.as_ref(),
            self.sources.available(package_id).is_some(),
        )?;
        self.store.deactivate(package_id).await?;
        if plan.preserve_unavailable {
            self.store
                .mark_failure(package_id, "unavailable", "application_source_unavailable")
                .await?;
        }
        self.status().await
    }

    async fn discover_skills(&self) -> Option<SkillDiscovery> {
        let (Some(runtime), Some(workspace)) = (&self.runtime, &self.runtime_workspace) else {
            return None;
        };
        match runtime
            .request(
                "skills/list",
                json!({"cwds": [workspace], "forceReload": true}),
            )
            .await
        {
            Ok(response) => Some(SkillDiscovery::Observed {
                ids: discovered_skill_ids(&response).into_iter().collect(),
                errors_empty: discovery_errors_empty(&response),
            }),
            Err(_) => Some(SkillDiscovery::Unavailable),
        }
    }

    fn summary(
        &self,
        record: InstallationRecord,
        discovery: Option<&SkillDiscovery>,
    ) -> CopilotInstallationSummary {
        let source_available = self.sources.available(&record.package_id).is_some();
        let configured = record.desired_active
            && record.configured_revision.as_deref() == Some(record.source_revision.as_str())
            && source_available;
        let mut discovered = Vec::new();
        let mut state = if let Some(kind) = record.last_failure_kind.as_deref() {
            if kind == "unavailable" {
                CopilotInstallationState::Unavailable
            } else {
                CopilotInstallationState::Failed
            }
        } else if !source_available {
            CopilotInstallationState::Unavailable
        } else if configured {
            CopilotInstallationState::Configured
        } else {
            CopilotInstallationState::Installed
        };
        if configured {
            if let (Some(assets), Some(discovery)) =
                (self.sources.available(&record.package_id), discovery)
            {
                match discovery {
                    SkillDiscovery::Unavailable => state = CopilotInstallationState::Unavailable,
                    SkillDiscovery::Observed { ids, errors_empty } => {
                        let package_skills = assets.skill_ids();
                        discovered = package_skills
                            .iter()
                            .filter(|id| ids.contains(*id))
                            .cloned()
                            .collect();
                        if !errors_empty {
                            state = CopilotInstallationState::Failed;
                        } else if discovered.len() != package_skills.len() {
                            state = CopilotInstallationState::Failed;
                        }
                    }
                }
            }
        }
        let restart_required = (!record.desired_active
            && (!record.managed_skill_ids.is_empty() || !record.managed_agent_role_ids.is_empty()))
            || (record.desired_active && !configured);
        CopilotInstallationSummary {
            package_id: record.package_id,
            source_revision: record.source_revision,
            active: record.desired_active,
            state,
            restart_required,
            managed_skill_ids: record.managed_skill_ids,
            managed_agent_role_ids: record.managed_agent_role_ids,
            runtime_discovered_skill_ids: discovered,
            failure_code: record.last_failure_code,
        }
    }
}

enum SkillDiscovery {
    Observed {
        ids: BTreeSet<String>,
        errors_empty: bool,
    },
    Unavailable,
}

fn discovered_skill_ids(response: &Value) -> Vec<String> {
    let mut ids = response["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| entry["skills"].as_array().into_iter().flatten())
        .filter(|skill| skill["scope"].as_str() == Some("user"))
        .filter_map(|skill| skill["name"].as_str().map(str::to_string))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn source_revision_requires_refresh(persisted: &str, registered: &str) -> bool {
    persisted != registered
}

fn discovery_errors_empty(response: &Value) -> bool {
    response["data"].as_array().is_some_and(|entries| {
        entries
            .iter()
            .all(|entry| entry["errors"].as_array().is_some_and(Vec::is_empty))
    })
}

fn validate_id(id: &str) -> Result<(), CopilotInstallationError> {
    let valid = !id.is_empty()
        && id.len() <= 96
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
        && id
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && id
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(CopilotInstallationError::InvalidSource(
            "invalid Copilot package id".into(),
        ))
    }
}

#[derive(Debug, Error)]
pub(crate) enum CopilotInstallationError {
    #[error("invalid application Copilot source: {0}")]
    InvalidSource(String),
    #[error("Copilot package was not found")]
    NotFound,
    #[error("Copilot package is unavailable")]
    Unavailable,
    #[error("Copilot package selection is invalid")]
    InvalidSelection,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Package(#[from] CopilotPackageError),
    #[error(transparent)]
    StartupFile(#[from] open_web_codex_profile_host::ProfileStartupFileError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery_contracts::{DeliveryContract, DeliveryKind};

    fn installation_record(
        desired_active: bool,
        configured_revision: Option<&str>,
        managed_skill_ids: &[&str],
        managed_agent_role_ids: &[&str],
        failure: Option<(&str, &str)>,
    ) -> InstallationRecord {
        InstallationRecord {
            package_id: "test-package".to_string(),
            desired_active,
            source_revision: "a".repeat(64),
            configured_revision: configured_revision.map(str::to_string),
            managed_skill_ids: managed_skill_ids
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            managed_agent_role_ids: managed_agent_role_ids
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            last_failure_kind: failure.map(|(kind, _)| kind.to_string()),
            last_failure_code: failure.map(|(_, code)| code.to_string()),
        }
    }

    fn lifecycle_delivery_registry() -> DeliveryRegistry {
        DeliveryRegistry::new(vec![DeliveryContract {
            id: "lifecycle-card".to_string(),
            server: "lifecycle_server".to_string(),
            tool: "publish_lifecycle_card".to_string(),
            kind: DeliveryKind::InlineGeoJsonMapCard,
            schema: "map.v3".to_string(),
            mime_type: "application/vnd.open-web-codex.map-card+json".to_string(),
            display_name: "Lifecycle card".to_string(),
        }])
        .expect("lifecycle delivery registry")
    }

    fn registry_with_assets(assets: CopilotPackageAssets) -> CopilotSourceRegistry {
        CopilotSourceRegistry {
            sources: Arc::new(BTreeMap::from([(
                assets.id().to_string(),
                RegisteredSource::Available(Arc::new(assets)),
            )])),
        }
    }

    #[test]
    fn official_skill_projection_requires_empty_discovery_errors() {
        let response = json!({"data": [{"skills": [{"name": "alpha", "scope": "user"}, {"name": "repo-collision", "scope": "repo"}], "errors": []}]});
        assert_eq!(discovered_skill_ids(&response), vec!["alpha"]);
        assert!(discovery_errors_empty(&response));
        assert!(!discovery_errors_empty(
            &json!({"data": [{"skills": [], "errors": [{}]}]})
        ));
    }

    #[test]
    fn skill_discovery_intersects_each_package_declared_skill_ids() {
        let response = json!({"data": [{"skills": [
            {"name": "alpha", "scope": "user"},
            {"name": "unrelated-package-skill", "scope": "user"}
        ], "errors": []}]});
        let discovery = SkillDiscovery::Observed {
            ids: discovered_skill_ids(&response).into_iter().collect(),
            errors_empty: discovery_errors_empty(&response),
        };
        let SkillDiscovery::Observed { ids, errors_empty } = discovery else {
            panic!("expected observed discovery");
        };
        assert!(errors_empty);
        let package_skills = ["alpha", "package-only"];
        let projected = package_skills
            .iter()
            .filter(|id| ids.contains(**id))
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(projected, vec!["alpha"]);
    }

    #[test]
    fn application_source_ids_are_typed_not_paths() {
        assert!(validate_id("warehouse-network").is_ok());
        assert!(validate_id("../warehouse").is_err());
        assert!(validate_id("Warehouse").is_err());
    }

    #[test]
    fn trusted_root_discovery_enumerates_each_copilot_manifest_without_defaults() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("repository root")
            .to_path_buf();
        let prepared = tempfile::tempdir().expect("empty prepared root");
        let build_store = tempfile::tempdir().expect("empty build store root");

        let registry = CopilotSourceRegistry::discover(
            &repository.join("copilots"),
            prepared.path(),
            build_store.path(),
        )
        .expect("discover trusted Copilot root");
        let ids = registry
            .summaries()
            .into_iter()
            .map(|package| package.package_id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec!["meeting-action-review", "warehouse-network-single-agent",]
        );
    }

    #[test]
    fn failure_update_preserves_last_configured_revision_and_owned_destinations() {
        assert!(!MARK_FAILURE_SQL.contains("configured_revision"));
        assert!(!MARK_FAILURE_SQL.contains("managed_skill_ids"));
        assert!(!MARK_FAILURE_SQL.contains("managed_agent_role_ids"));
    }

    #[test]
    fn unavailable_cleanup_clears_only_runtime_configuration_after_host_success() {
        assert!(MARK_UNAVAILABLE_CLEANED_SQL.contains("configured_revision = NULL"));
        assert!(MARK_UNAVAILABLE_CLEANED_SQL.contains("managed_skill_ids = '{}'"));
        assert!(MARK_UNAVAILABLE_CLEANED_SQL.contains("managed_agent_role_ids = '{}'"));
        assert!(MARK_UNAVAILABLE_CLEANED_SQL.contains("last_failure_kind = 'unavailable'"));
        assert!(!MARK_UNAVAILABLE_CLEANED_SQL.contains("desired_active ="));
        assert!(!MARK_UNAVAILABLE_CLEANED_SQL.contains("source_revision ="));
    }

    #[test]
    fn source_present_and_absent_deactivation_use_persisted_installation_authority() {
        let record = installation_record(
            true,
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            &["managed-skill"],
            &["managed_role"],
            None,
        );
        assert_eq!(
            plan_deactivation(Some(&record), true).expect("source-present plan"),
            DeactivationPlan {
                preserve_unavailable: false,
            }
        );
        assert_eq!(
            plan_deactivation(Some(&record), false).expect("source-absent plan"),
            DeactivationPlan {
                preserve_unavailable: true,
            }
        );
        assert!(matches!(
            plan_deactivation(None, true),
            Err(CopilotInstallationError::NotFound)
        ));
    }

    #[tokio::test]
    async fn source_absence_projects_unavailable_and_restart_required_from_durable_state() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://localhost/open_web_codex")
            .expect("lazy PostgreSQL pool");
        let service = CopilotInstallationService::new(
            CopilotInstallationStore::new(pool, "test-runtime"),
            CopilotSourceRegistry::default(),
            None,
            None,
        );
        let configured = service.summary(
            installation_record(
                true,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                &["managed-skill"],
                &["managed_role"],
                Some(("unavailable", "application_source_unavailable")),
            ),
            None,
        );
        assert_eq!(configured.state, CopilotInstallationState::Unavailable);
        assert!(configured.restart_required);
        assert_eq!(configured.managed_skill_ids, vec!["managed-skill"]);

        let cleaned_active = service.summary(
            installation_record(
                true,
                None,
                &[],
                &[],
                Some(("unavailable", "application_source_unavailable")),
            ),
            None,
        );
        assert_eq!(cleaned_active.state, CopilotInstallationState::Unavailable);
        assert!(cleaned_active.restart_required);
        assert!(cleaned_active.managed_skill_ids.is_empty());

        let cleaned_inactive = service.summary(
            installation_record(
                false,
                None,
                &[],
                &[],
                Some(("unavailable", "application_source_unavailable")),
            ),
            None,
        );
        assert_eq!(
            cleaned_inactive.state,
            CopilotInstallationState::Unavailable
        );
        assert!(!cleaned_inactive.restart_required);
    }

    #[test]
    fn changed_registered_source_revision_requires_cold_start_refresh() {
        assert!(source_revision_requires_refresh(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ));
        assert!(!source_revision_requires_refresh(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ));
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
    async fn persisted_installation_converges_revision_failure_and_deactivation() {
        let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("connect test database");
        open_web_codex_platform_store::migrate::run(&pool)
            .await
            .expect("migrate test database");
        crate::ensure_local_owner(&pool).await.expect("local owner");
        let runtime_key = format!("copilot-installation-test-{}", uuid::Uuid::now_v7());
        crate::ensure_transitional_profile_binding(&pool, &runtime_key, "Copilot Test Profile")
            .await
            .expect("test Profile");
        let store = CopilotInstallationStore::new(pool, &runtime_key);
        let first = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let second = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        store
            .activate_values("test-package", first)
            .await
            .expect("activate");
        store
            .mark_configured(
                "test-package",
                Some(first),
                &["managed-skill".to_string()],
                &["managed_role".to_string()],
            )
            .await
            .expect("mark configured");
        store
            .refresh_source_revision("test-package", second)
            .await
            .expect("follow registered source");
        let configure_second = ColdStartCompletion {
            changes: vec![ColdStartChange::Configure {
                package_id: "test-package".to_string(),
                revision: second.to_string(),
                skill_ids: vec!["next-skill".to_string()],
                role_ids: vec!["next_role".to_string()],
            }],
        };
        configure_second
            .mark_host_failed(&store, "profile_host_start_failed")
            .await
            .expect("record failed Host start");
        let failed = store
            .load_package("test-package")
            .await
            .expect("load failure")
            .expect("record");
        assert_eq!(failed.source_revision, second);
        assert_eq!(failed.configured_revision.as_deref(), Some(first));
        assert_eq!(failed.managed_skill_ids, vec!["managed-skill"]);
        assert_eq!(failed.managed_agent_role_ids, vec!["managed_role"]);

        configure_second
            .mark_host_ready(&store)
            .await
            .expect("record successful Host start");
        let configured = store
            .load_package("test-package")
            .await
            .expect("load ready")
            .expect("record");
        assert_eq!(configured.configured_revision.as_deref(), Some(second));
        assert_eq!(configured.managed_skill_ids, vec!["next-skill"]);
        assert_eq!(configured.managed_agent_role_ids, vec!["next_role"]);

        let profile = tempfile::tempdir().expect("temporary Profile");
        let missing_sources = CopilotSourceRegistry::default();
        let missing = cold_start_composition(&store, &missing_sources, profile.path())
            .await
            .expect("compose missing source cleanup");
        assert!(missing.startup_files.is_empty());
        assert_eq!(missing.removed_startup_files.len(), 2);
        assert!(missing.root_execution_configs.is_empty());
        assert!(missing.active_package_ids.is_empty());
        assert!(missing
            .deliveries
            .for_item(
                json!({"server": "lifecycle_server", "tool": "publish_lifecycle_card"})
                    .as_object()
                    .expect("delivery item"),
            )
            .is_none());
        missing
            .completion
            .mark_host_failed(&store, "profile_host_start_failed")
            .await
            .expect("preserve unavailable cleanup after Host failure");
        let cleanup_failed = store
            .load_package("test-package")
            .await
            .expect("load cleanup failure")
            .expect("record");
        assert_eq!(cleanup_failed.configured_revision.as_deref(), Some(second));
        assert_eq!(cleanup_failed.managed_skill_ids, vec!["next-skill"]);
        assert_eq!(cleanup_failed.managed_agent_role_ids, vec!["next_role"]);
        assert_eq!(
            cleanup_failed.last_failure_code.as_deref(),
            Some("application_source_unavailable")
        );
        missing
            .completion
            .mark_host_ready(&store)
            .await
            .expect("complete unavailable cleanup");
        let cleaned = store
            .load_package("test-package")
            .await
            .expect("load cleaned unavailable record")
            .expect("record");
        assert!(cleaned.desired_active);
        assert!(cleaned.configured_revision.is_none());
        assert!(cleaned.managed_skill_ids.is_empty());
        assert!(cleaned.managed_agent_role_ids.is_empty());
        assert_eq!(
            cleaned.last_failure_code.as_deref(),
            Some("application_source_unavailable")
        );

        let third = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let available_sources = registry_with_assets(CopilotPackageAssets::lifecycle_test_assets(
            "test-package",
            third,
            lifecycle_delivery_registry(),
        ));
        let restored = cold_start_composition(&store, &available_sources, profile.path())
            .await
            .expect("compose restored source");
        assert_eq!(restored.startup_files.len(), 1);
        assert!(restored.removed_startup_files.is_empty());
        assert_eq!(restored.root_execution_configs.len(), 1);
        assert_eq!(restored.active_package_ids, vec!["test-package"]);
        assert!(restored
            .deliveries
            .for_item(
                json!({"server": "lifecycle_server", "tool": "publish_lifecycle_card"})
                    .as_object()
                    .expect("delivery item"),
            )
            .is_some());
        restored
            .completion
            .mark_host_ready(&store)
            .await
            .expect("complete restored source");
        let restored_record = store
            .load_package("test-package")
            .await
            .expect("load restored record")
            .expect("record");
        assert_eq!(restored_record.source_revision, third);
        assert_eq!(restored_record.configured_revision.as_deref(), Some(third));
        assert_eq!(restored_record.managed_skill_ids, vec!["lifecycle-root"]);
        assert!(restored_record.last_failure_code.is_none());

        let absent_service = CopilotInstallationService::new(
            store.clone(),
            CopilotSourceRegistry::default(),
            None,
            None,
        );
        let absent_deactivated = absent_service
            .deactivate("test-package")
            .await
            .expect("deactivate from persisted record without source");
        let absent_summary = absent_deactivated
            .installations
            .into_iter()
            .find(|installation| installation.package_id == "test-package")
            .expect("source-absent installation summary");
        assert!(!absent_summary.active);
        assert_eq!(absent_summary.state, CopilotInstallationState::Unavailable);
        assert!(absent_summary.restart_required);

        let absent_cleanup = cold_start_composition(&store, &missing_sources, profile.path())
            .await
            .expect("compose source-absent deactivation");
        assert_eq!(absent_cleanup.removed_startup_files.len(), 1);
        assert!(absent_cleanup
            .deliveries
            .for_item(
                json!({"server": "lifecycle_server", "tool": "publish_lifecycle_card"})
                    .as_object()
                    .expect("delivery item"),
            )
            .is_none());
        absent_cleanup
            .completion
            .mark_host_ready(&store)
            .await
            .expect("complete source-absent deactivation");

        let available_service =
            CopilotInstallationService::new(store.clone(), available_sources.clone(), None, None);
        available_service
            .activate("test-package")
            .await
            .expect("reactivate source-present package");
        let reconfigured = cold_start_composition(&store, &available_sources, profile.path())
            .await
            .expect("compose reactivated source");
        reconfigured
            .completion
            .mark_host_ready(&store)
            .await
            .expect("complete reactivation");
        let present_deactivated = available_service
            .deactivate("test-package")
            .await
            .expect("deactivate source-present package");
        let present_summary = present_deactivated
            .installations
            .into_iter()
            .find(|installation| installation.package_id == "test-package")
            .expect("source-present installation summary");
        assert!(!present_summary.active);
        assert_eq!(present_summary.state, CopilotInstallationState::Installed);
        assert!(present_summary.restart_required);
        let present_cleanup = cold_start_composition(&store, &available_sources, profile.path())
            .await
            .expect("compose source-present deactivation");
        assert!(present_cleanup
            .deliveries
            .for_item(
                json!({"server": "lifecycle_server", "tool": "publish_lifecycle_card"})
                    .as_object()
                    .expect("delivery item"),
            )
            .is_none());
        present_cleanup
            .completion
            .mark_host_ready(&store)
            .await
            .expect("complete source-present deactivation");
        let inactive = store
            .load_package("test-package")
            .await
            .expect("load inactive")
            .expect("record");
        assert!(!inactive.desired_active);
        assert!(inactive.configured_revision.is_none());
        assert!(inactive.managed_skill_ids.is_empty());
        assert!(inactive.managed_agent_role_ids.is_empty());
    }
}
