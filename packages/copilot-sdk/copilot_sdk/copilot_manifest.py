"""Static validation for a composed Copilot package manifest."""

from __future__ import annotations

import hashlib
import json
import os
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import tomllib

from .tool_runtime_manifest import (
    ToolRuntimeManifestError,
    load_tool_runtime_manifest,
)

ROLE_MCP_SERVER_POLICY_FIELDS = frozenset(
    {
        "enabled",
        "required",
        "omit_tools_from",
        "default_tools_approval_mode",
        "tools",
    }
)
MAX_DELIVERIES = 32
MAX_DELIVERY_TEXT = 256
MAX_DELIVERY_SCHEMA_BYTES = 1024 * 1024
MAX_TESTS = 16


class CopilotPackageError(ValueError):
    """A typed validation failure for a Copilot package."""

    def __init__(self, code: str, relative_path: str, message: str) -> None:
        self.code = code
        self.relative_path = relative_path
        self.message = message
        super().__init__(f"{code}: {relative_path}: {message}")


@dataclass(frozen=True)
class CopilotPackageSummary:
    """Stable, bounded result returned after manifest validation."""

    id: str
    display_name: str
    skill_ids: tuple[str, ...]
    agent_ids: tuple[str, ...]
    tool_ids: tuple[str, ...]
    test_ids: tuple[str, ...]
    composition_descriptor_sha256: str
    deliveries: tuple[CopilotDelivery, ...] = ()


@dataclass(frozen=True)
class CopilotDelivery:
    """One package-owned declaration using a versioned Platform envelope."""

    id: str
    server: str
    tool: str
    kind: str
    schema: str
    mime_type: str
    display_name: str
    verifier_kind: str | None = None
    verifier_value: str | dict[str, Any] | None = None


@dataclass(frozen=True)
class CopilotTestCase:
    """One bounded native Runtime acceptance case declared by the author."""

    id: str
    prompt: str
    target_kind: str
    target_agent: str | None
    tool: str
    server: str
    tool_name: str
    arguments: dict[str, Any]
    expected_structured_content: dict[str, Any]


def validate_copilot_package(
    source_root: Path,
    manifest_path: Path | None = None,
    tool_registry_root: Path | None = None,
) -> CopilotPackageSummary:
    """Validate and summarize a composed Copilot package."""

    root = Path(source_root)
    if not root.exists():
        _fail("source_root_invalid", ".", "source root does not exist")
    if root.is_symlink():
        _fail("unsafe_symlink", ".", "source root must not be a symlink")
    if not root.is_dir():
        _fail("source_root_invalid", ".", "source root must be a directory")
    root = root.resolve(strict=True)

    manifest_relative = _manifest_relative_path(root, manifest_path)
    manifest_file = _safe_package_path(root, manifest_relative, kind="file")
    manifest = _load_toml(manifest_file, manifest_relative.as_posix())

    schema_version = _required(manifest, "schema_version", "copilot.toml")
    if type(schema_version) is not int:
        _fail("invalid_type", "schema_version", "must be an integer")
    if schema_version != 1:
        _fail("schema_version", "schema_version", "must equal 1")

    copilot_id = _required_string(manifest, "id", "copilot.toml")
    display_name = _required_string(manifest, "display_name", "copilot.toml")
    skills = _component_entries(manifest, "skills", "path")
    agents = _component_entries(manifest, "agents", "role")
    tools = _tool_entries(
        manifest,
        source_root=root,
        tool_registry_root=tool_registry_root,
    )

    skill_ids = tuple(entry["id"] for entry in skills)
    agent_ids = tuple(entry["id"] for entry in agents)
    tool_ids = tuple(entry["id"] for entry in tools)
    root_config = _required(manifest, "root", "copilot.toml")
    if not isinstance(root_config, dict):
        _fail("invalid_type", "root", "must be a table")
    unknown_root = sorted(set(root_config) - {"skill", "agent", "task_skills"})
    if unknown_root:
        _fail(
            "invalid_field",
            f"root.{unknown_root[0]}",
            "field is not part of schema v1",
        )
    root_skill = _required_string(root_config, "skill", "root")
    task_skills = _required_string(root_config, "task_skills", "root")
    if task_skills not in {"none", "all"}:
        _fail(
            "invalid_field",
            "root.task_skills",
            "must equal 'none' or 'all'",
        )
    if root_skill not in skill_ids:
        _fail(
            "missing_reference",
            "root.skill",
            f"references undeclared skill {root_skill!r}",
        )
    root_agent = root_config.get("agent")
    if root_agent is not None:
        if not isinstance(root_agent, str) or not root_agent.strip():
            _fail("invalid_type", "root.agent", "must be a non-empty string")
        if root_agent not in agent_ids:
            _fail(
                "missing_reference",
                "root.agent",
                f"references undeclared agent {root_agent!r}",
            )
    tests = _test_entries(
        manifest,
        agent_ids=agent_ids,
        tool_ids=tool_ids,
        root_agent=root_agent,
    )

    author_files: dict[str, Path] = {
        manifest_relative.as_posix(): manifest_file,
    }

    for index, entry in enumerate(skills):
        skill_id = entry["id"]
        relative = _authored_relative_path(entry["path"], f"skills[{index}].path")
        skill_root = _safe_package_path(root, relative, kind="directory")
        skill_file_relative = relative / "SKILL.md"
        skill_file = _safe_package_path(root, skill_file_relative, kind="file")
        frontmatter = _skill_frontmatter(
            skill_file, skill_file_relative.as_posix()
        )
        if frontmatter.name != skill_id:
            _fail(
                "frontmatter_mismatch",
                skill_file_relative.as_posix(),
                f"frontmatter name {frontmatter.name!r} does not match {skill_id!r}",
            )
        # Retain the directory check as part of the explicit package contract.
        del skill_root
        author_files[skill_file_relative.as_posix()] = skill_file

    agent_tool_policies: dict[str, dict[str, dict[str, None]]] = {}
    for index, entry in enumerate(agents):
        relative = _authored_relative_path(entry["role"], f"agents[{index}].role")
        role_file = _safe_package_path(root, relative, kind="file")
        role = _load_toml(role_file, relative.as_posix())
        role_name = _required_string(role, "name", relative.as_posix())
        if role_name != entry["id"]:
            _fail(
                "missing_reference",
                f"agents[{index}].id",
                f"agent id {entry['id']!r} does not match role name {role_name!r}",
            )
        _validate_role_references(role, relative.as_posix(), skill_ids, tool_ids)
        agent_tool_policies[entry["id"]] = _role_tool_policies(
            role, relative.as_posix(), tool_ids
        )
        author_files[relative.as_posix()] = role_file

    for index, test in enumerate(tests):
        policy_agent = root_agent if test.target_kind == "root" else test.target_agent
        assert policy_agent is not None
        tool_policy = agent_tool_policies[policy_agent].get(test.tool)
        if tool_policy is None:
            _fail(
                "missing_reference",
                f"tests[{index}].tool",
                f"target {policy_agent!r} does not enable declared tool {test.tool!r}",
            )
        policy = tool_policy.get(test.server)
        if policy is None and test.server not in tool_policy:
            _fail(
                "missing_reference",
                f"tests[{index}].server",
                f"target {policy_agent!r} does not enable MCP server {test.server!r} "
                f"from capability root {test.tool!r}",
            )
        if policy is not None and test.tool_name not in policy:
            _fail(
                "missing_reference",
                f"tests[{index}].tool_name",
                f"target {policy_agent!r} does not enable tool method {test.tool_name!r}",
            )

    tool_server_ids: dict[str, set[str]] = {}
    tool_sources: dict[str, dict[str, Any]] = {}
    server_owners: dict[str, str] = {}
    for index, entry in enumerate(tools):
        owner_root = entry["owner_root"]
        tool_sources[entry["id"]] = entry
        relative = _authored_relative_path(entry["root"], f"tools[{index}].root")
        tool_root = _safe_package_path(owner_root, relative, kind="directory")
        runtime_relative = _authored_relative_path(
            entry["runtime"], f"tools[{index}].runtime"
        )
        try:
            runtime = load_tool_runtime_manifest(owner_root, tool_root, runtime_relative)
        except ToolRuntimeManifestError as error:
            _fail(error.code, error.relative_path, error.message)
        tool_server_ids[entry["id"]] = {server.id for server in runtime.servers}
        for server_id in tool_server_ids[entry["id"]]:
            previous = server_owners.get(server_id)
            if previous is not None:
                _fail(
                    "duplicate_id",
                    f"tools[{index}].runtime",
                    f"MCP server {server_id!r} is declared by both capability roots "
                    f"{previous!r} and {entry['id']!r}",
                )
            server_owners[server_id] = entry["id"]
        author_prefix = entry["author_prefix"]
        author_files[f"{author_prefix}{runtime_relative.as_posix()}"] = runtime.path
        owner_manifest = entry.get("owner_manifest")
        if owner_manifest is not None:
            author_files[f"{author_prefix}tool.toml"] = owner_manifest
        for dependency in runtime.dependencies:
            for dependency_file in (dependency.manifest, dependency.lock):
                dependency_relative = dependency_file.relative_to(owner_root).as_posix()
                author_files[f"{author_prefix}{dependency_relative}"] = dependency_file

    for index, test in enumerate(tests):
        if test.server not in tool_server_ids[test.tool]:
            _fail(
                "missing_reference",
                f"tests[{index}].server",
                f"capability root {test.tool!r} does not declare MCP server {test.server!r}",
            )

    deliveries = _delivery_entries(
        manifest,
        root,
        server_owners,
        author_files,
        tool_sources,
    )

    digest = hashlib.sha256()
    for relative_path in sorted(author_files):
        contents = author_files[relative_path].read_bytes()
        path_bytes = relative_path.encode("utf-8")
        digest.update(len(path_bytes).to_bytes(8, "big"))
        digest.update(path_bytes)
        digest.update(len(contents).to_bytes(8, "big"))
        digest.update(contents)

    return CopilotPackageSummary(
        id=copilot_id,
        display_name=display_name,
        skill_ids=skill_ids,
        agent_ids=agent_ids,
        tool_ids=tool_ids,
        test_ids=tuple(test.id for test in tests),
        composition_descriptor_sha256=digest.hexdigest(),
        deliveries=tuple(deliveries),
    )


def load_copilot_test_cases(
    source_root: Path,
    manifest_path: Path = Path("copilot.toml"),
    tool_registry_root: Path | None = None,
) -> tuple[CopilotTestCase, ...]:
    """Load tests only after the complete package passes static validation."""

    summary = validate_copilot_package(
        source_root,
        manifest_path,
        tool_registry_root=tool_registry_root,
    )
    root = Path(source_root).resolve(strict=True)
    manifest_relative = _manifest_relative_path(root, manifest_path)
    manifest = _load_toml(root / manifest_relative, manifest_relative.as_posix())
    tests = _test_entries(
        manifest,
        agent_ids=summary.agent_ids,
        tool_ids=summary.tool_ids,
        root_agent=(
            manifest.get("root", {}).get("agent")
            if isinstance(manifest.get("root"), dict)
            else None
        ),
    )
    return tuple(tests)


def _fail(code: str, relative_path: str, message: str) -> None:
    raise CopilotPackageError(code, relative_path, message)


def _manifest_relative_path(root: Path, manifest_path: Path | None) -> Path:
    candidate = Path("copilot.toml") if manifest_path is None else Path(manifest_path)
    if candidate.is_absolute():
        _fail("invalid_path", str(candidate), "manifest path must be source-root-relative")
    return _authored_relative_path(candidate, "manifest_path")


def _authored_relative_path(value: Any, field: str) -> Path:
    if not isinstance(value, (str, os.PathLike)):
        _fail("invalid_type", field, "must be a relative path string")
    raw = os.fspath(value)
    if not isinstance(raw, str) or not raw:
        _fail("invalid_path", field, "must be a non-empty relative path")
    path = Path(raw)
    if path.is_absolute():
        _fail("invalid_path", field, "absolute paths are not allowed")
    if any(part == ".." for part in path.parts):
        _fail("invalid_path", field, "parent traversal is not allowed")
    if path == Path(".") or any(part in ("", ".") for part in path.parts):
        _fail("invalid_path", field, "path must identify a package entry")
    return path


def _safe_package_path(root: Path, relative: Path, *, kind: str) -> Path:
    current = root
    for part in relative.parts:
        current = current / part
        try:
            mode = current.lstat().st_mode
        except FileNotFoundError:
            _fail("missing_file", relative.as_posix(), f"{kind} does not exist")
        if stat.S_ISLNK(mode):
            _fail(
                "unsafe_symlink",
                relative.as_posix(),
                "symlinks are not allowed in package paths",
            )
    if kind == "file" and not current.is_file():
        _fail("invalid_type", relative.as_posix(), "must be a file")
    if kind == "directory" and not current.is_dir():
        _fail("invalid_type", relative.as_posix(), "must be a directory")
    return current


def _load_toml(path: Path, relative_path: str) -> dict[str, Any]:
    try:
        with path.open("rb") as handle:
            value = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as exc:
        _fail("invalid_toml", relative_path, str(exc))
    if not isinstance(value, dict):  # Defensive; tomllib currently always returns dict.
        _fail("invalid_type", relative_path, "TOML document must be a table")
    return value


def _load_json(path: Path, relative_path: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        _fail("invalid_type", relative_path, f"invalid JSON: {exc}")
    if not isinstance(value, dict):
        _fail("invalid_type", relative_path, "JSON document must be an object")
    return value


def _required(table: dict[str, Any], key: str, relative_path: str) -> Any:
    if key not in table:
        _fail("required_field", f"{relative_path}.{key}", "required field is missing")
    return table[key]


def _required_string(table: dict[str, Any], key: str, relative_path: str) -> str:
    value = _required(table, key, relative_path)
    if not isinstance(value, str) or not value.strip():
        _fail("invalid_type", f"{relative_path}.{key}", "must be a non-empty string")
    return value


def _component_entries(
    manifest: dict[str, Any], section: str, path_field: str
) -> list[dict[str, str]]:
    raw_entries = _required(manifest, section, "copilot.toml")
    if not isinstance(raw_entries, list):
        _fail("invalid_type", section, "must be an array of tables")
    entries: list[dict[str, str]] = []
    seen: set[str] = set()
    for index, raw_entry in enumerate(raw_entries):
        location = f"{section}[{index}]"
        if not isinstance(raw_entry, dict):
            _fail("invalid_type", location, "must be a table")
        entry_id = _required_string(raw_entry, "id", location)
        entry_path = _required_string(raw_entry, path_field, location)
        if entry_id in seen:
            _fail("duplicate_id", f"{location}.id", f"duplicate id {entry_id!r}")
        seen.add(entry_id)
        entries.append({"id": entry_id, path_field: entry_path})
    return entries


def _tool_entries(
    manifest: dict[str, Any],
    *,
    source_root: Path,
    tool_registry_root: Path | None,
) -> list[dict[str, Any]]:
    raw_entries = _required(manifest, "tools", "copilot.toml")
    if not isinstance(raw_entries, list):
        _fail("invalid_type", "tools", "must be an array of tables")
    entries: list[dict[str, Any]] = []
    seen: set[str] = set()
    for index, raw_entry in enumerate(raw_entries):
        location = f"tools[{index}]"
        if not isinstance(raw_entry, dict):
            _fail("invalid_type", location, "must be a table")
        local_fields = {"id", "root", "runtime"}
        shared_fields = {"id", "package"}
        fields = set(raw_entry)
        if fields <= local_fields:
            expected_fields = local_fields
            shared = False
        elif fields <= shared_fields:
            expected_fields = shared_fields
            shared = True
        else:
            expected_fields = local_fields | shared_fields
            shared = "package" in fields
        unknown = sorted(fields - expected_fields)
        if unknown:
            _fail("invalid_field", f"{location}.{unknown[0]}", "field is not part of schema v1")
        entry_id = _required_string(raw_entry, "id", location)
        if entry_id in seen:
            _fail("duplicate_id", f"{location}.id", f"duplicate id {entry_id!r}")
        seen.add(entry_id)
        if shared:
            if fields != shared_fields:
                _fail(
                    "invalid_field",
                    location,
                    "shared Tool references require exactly id and package",
                )
            package = _required_string(raw_entry, "package", location)
            resolved = _resolve_registered_tool(
                tool_registry_root,
                package,
                location,
            )
            entries.append(
                {
                    "id": entry_id,
                    "root": resolved[1],
                    "runtime": resolved[2],
                    "owner_root": resolved[0],
                    "owner_manifest": resolved[3],
                    "author_prefix": f"tools/{package}/",
                }
            )
        else:
            if fields != local_fields:
                _fail(
                    "invalid_field",
                    location,
                    "local Tool declarations require exactly id, root, and runtime",
                )
            entries.append(
                {
                    "id": entry_id,
                    "root": _required_string(raw_entry, "root", location),
                    "runtime": _required_string(raw_entry, "runtime", location),
                    "owner_root": source_root,
                    "owner_manifest": None,
                    "author_prefix": "",
                }
            )
    return entries


def _resolve_registered_tool(
    tool_registry_root: Path | None,
    package_id: str,
    location: str,
) -> tuple[Path, str, str, Path]:
    if tool_registry_root is None:
        _fail(
            "missing_reference",
            f"{location}.package",
            "shared Tool references require an explicit Tool registry root",
        )
    registry = Path(tool_registry_root)
    if not registry.exists() or registry.is_symlink() or not registry.is_dir():
        _fail("source_root_invalid", "tool_registry_root", "must be a non-symlink directory")
    registry = registry.resolve(strict=True)
    matches: list[tuple[Path, Path, dict[str, Any]]] = []
    for candidate in sorted(registry.iterdir(), key=lambda path: path.name):
        if candidate.is_symlink() or not candidate.is_dir():
            continue
        manifest_path = candidate / "tool.toml"
        if manifest_path.is_symlink() or not manifest_path.is_file():
            continue
        document = _load_toml(manifest_path, f"{candidate.name}/tool.toml")
        if document.get("id") == package_id:
            matches.append((candidate.resolve(strict=True), manifest_path, document))
    if len(matches) != 1:
        _fail(
            "missing_reference",
            f"{location}.package",
            f"registered Tool package {package_id!r} was not found exactly once",
        )
    owner_root, owner_manifest, document = matches[0]
    if set(document) != {"schema_version", "id", "runtime"}:
        _fail(
            "invalid_field",
            f"{location}.package",
            "Tool package manifest requires exactly schema_version, id, and runtime",
        )
    if document.get("schema_version") != 1:
        _fail("schema_version", f"{location}.package", "Tool package schema_version must equal 1")
    runtime = _required_string(document, "runtime", f"tool package {package_id}")
    relative_root = owner_root.relative_to(registry).as_posix()
    return (
        registry,
        relative_root,
        (Path(relative_root) / runtime).as_posix(),
        owner_manifest,
    )


def _delivery_entries(
    manifest: dict[str, Any],
    root: Path,
    server_owners: dict[str, str],
    author_files: dict[str, Path],
    tool_sources: dict[str, dict[str, Any]],
) -> list[CopilotDelivery]:
    raw_entries = manifest.get("deliveries", [])
    if not isinstance(raw_entries, list):
        _fail("invalid_type", "deliveries", "must be an array of tables")
    if len(raw_entries) > MAX_DELIVERIES:
        _fail("invalid_type", "deliveries", f"must not exceed {MAX_DELIVERIES} entries")
    deliveries: list[CopilotDelivery] = []
    seen_ids: set[str] = set()
    seen_producers: set[tuple[str, str]] = set()
    for index, raw_entry in enumerate(raw_entries):
        location = f"deliveries[{index}]"
        if not isinstance(raw_entry, dict):
            _fail("invalid_type", location, "must be a table")
        allowed = {
            "id", "server", "tool", "kind", "schema", "mime_type",
            "display_name", "content_verifier",
        }
        unknown = sorted(set(raw_entry) - allowed)
        if unknown:
            _fail("invalid_field", f"{location}.{unknown[0]}", "field is not part of schema v1")
        delivery_id = _required_string(raw_entry, "id", location)
        if delivery_id in seen_ids:
            _fail("duplicate_id", f"{location}.id", f"duplicate id {delivery_id!r}")
        seen_ids.add(delivery_id)
        server = _required_string(raw_entry, "server", location)
        tool = _required_string(raw_entry, "tool", location)
        if server not in server_owners:
            _fail("missing_reference", f"{location}.server", f"references undeclared MCP server {server!r}")
        producer = (server, tool)
        if producer in seen_producers:
            _fail("duplicate_id", f"{location}.tool", f"duplicate delivery producer {server!r}/{tool!r}")
        seen_producers.add(producer)
        kind = _required_string(raw_entry, "kind", location)
        if kind not in ("workspace_artifact", "inline_geojson_map_card"):
            _fail("invalid_type", f"{location}.kind", "must be workspace_artifact or inline_geojson_map_card")
        schema = _required_string(raw_entry, "schema", location)
        mime_type = _required_string(raw_entry, "mime_type", location)
        display_name = _required_string(raw_entry, "display_name", location)
        for field_name, value in (
            ("id", delivery_id), ("server", server), ("tool", tool),
            ("schema", schema), ("mime_type", mime_type), ("display_name", display_name),
        ):
            if len(value) > MAX_DELIVERY_TEXT:
                _fail("invalid_type", f"{location}.{field_name}", f"must not exceed {MAX_DELIVERY_TEXT} characters")
        verifier = raw_entry.get("content_verifier")
        verifier_kind: str | None = None
        verifier_value: str | dict[str, Any] | None = None
        if kind == "workspace_artifact":
            if not isinstance(verifier, dict):
                _fail("required_field", f"{location}.content_verifier", "workspace_artifact requires a content verifier table")
            verifier_unknown = sorted(
                set(verifier) - {"kind", "schema_path", "marker", "tool"}
            )
            if verifier_unknown:
                _fail("invalid_field", f"{location}.content_verifier.{verifier_unknown[0]}", "field is not part of schema v1")
            verifier_kind = _required_string(verifier, "kind", f"{location}.content_verifier")
            if verifier_kind == "json_schema":
                expected = {"kind", "schema_path"}
                verifier_tool = verifier.get("tool")
                if verifier_tool is not None:
                    expected.add("tool")
                if set(verifier) != expected:
                    _fail("invalid_field", f"{location}.content_verifier", "json_schema requires kind, schema_path, and optional tool")
                schema_relative = _authored_relative_path(
                    _required_string(verifier, "schema_path", f"{location}.content_verifier"),
                    f"{location}.content_verifier.schema_path",
                )
                if verifier_tool is None:
                    schema_root = root
                    relative = schema_relative
                    author_path = relative.as_posix()
                else:
                    if not isinstance(verifier_tool, str) or verifier_tool not in tool_sources:
                        _fail(
                            "missing_reference",
                            f"{location}.content_verifier.tool",
                            "must reference a declared Tool",
                        )
                    source = tool_sources[verifier_tool]
                    schema_root = source["owner_root"]
                    relative = Path(source["root"]) / schema_relative
                    author_path = f"{source['author_prefix']}{relative.as_posix()}"
                schema_file = _safe_package_path(schema_root, relative, kind="file")
                if schema_file.stat().st_size > MAX_DELIVERY_SCHEMA_BYTES:
                    _fail("invalid_type", relative.as_posix(), "delivery JSON Schema must not exceed 1 MiB")
                schema_document = _load_json(schema_file, relative.as_posix())
                _validate_bounded_schema(schema_document, relative.as_posix())
                if schema_document.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
                    _fail("invalid_type", relative.as_posix(), "must declare JSON Schema draft 2020-12")
                verifier_value = schema_document
                author_files[author_path] = schema_file
            elif verifier_kind == "markdown_marker":
                if set(verifier) != {"kind", "marker"}:
                    _fail("invalid_field", f"{location}.content_verifier", "markdown_marker requires only kind and marker")
                marker = _required_string(verifier, "marker", f"{location}.content_verifier")
                if len(marker) > 256 or "\n" in marker or "\r" in marker:
                    _fail("invalid_type", f"{location}.content_verifier.marker", "must be one line of at most 256 characters")
                verifier_value = marker
            else:
                _fail("invalid_type", f"{location}.content_verifier.kind", "must be json_schema or markdown_marker")
        elif verifier is not None:
            _fail("invalid_field", f"{location}.content_verifier", "inline_geojson_map_card uses its versioned Platform verifier")
        if kind == "inline_geojson_map_card" and mime_type != "application/vnd.open-web-codex.map-card+json":
            _fail("invalid_type", f"{location}.mime_type", "inline_geojson_map_card requires its versioned media type")
        deliveries.append(CopilotDelivery(
            id=delivery_id,
            server=server,
            tool=tool,
            kind=kind,
            schema=schema,
            mime_type=mime_type,
            display_name=display_name,
            verifier_kind=verifier_kind,
            verifier_value=verifier_value,
        ))
    return deliveries


def _validate_bounded_schema(value: Any, location: str) -> None:
    nodes = 0

    def visit(current: Any, depth: int) -> None:
        nonlocal nodes
        nodes += 1
        if nodes > 20_000 or depth > 32:
            _fail("invalid_type", location, "delivery JSON Schema exceeds structural limits")
        if isinstance(current, dict):
            for key, child in current.items():
                if not isinstance(key, str) or len(key) > 2_000:
                    _fail("invalid_type", location, "delivery JSON Schema has an invalid key")
                visit(child, depth + 1)
        elif isinstance(current, list):
            for child in current:
                visit(child, depth + 1)
        elif isinstance(current, str) and len(current) > 64 * 1024:
            _fail("invalid_type", location, "delivery JSON Schema contains an oversized string")

    visit(value, 0)


def _test_entries(
    manifest: dict[str, Any],
    *,
    agent_ids: tuple[str, ...],
    tool_ids: tuple[str, ...],
    root_agent: str | None,
) -> list[CopilotTestCase]:
    raw_tests = manifest.get("tests", [])
    if not isinstance(raw_tests, list):
        _fail("invalid_type", "tests", "must be an array of tables")
    if len(raw_tests) > MAX_TESTS:
        _fail("invalid_type", "tests", f"must not exceed {MAX_TESTS} entries")
    tests: list[CopilotTestCase] = []
    seen: set[str] = set()
    for index, raw_test in enumerate(raw_tests):
        location = f"tests[{index}]"
        if not isinstance(raw_test, dict):
            _fail("invalid_type", location, "must be a table")
        unknown = sorted(
            set(raw_test)
            - {"id", "prompt", "target", "tool", "server", "tool_name", "arguments", "expect"}
        )
        if unknown:
            _fail("invalid_field", f"{location}.{unknown[0]}", "field is not part of schema v1")
        test_id = _required_string(raw_test, "id", location)
        if test_id in seen:
            _fail("duplicate_id", f"{location}.id", f"duplicate id {test_id!r}")
        seen.add(test_id)
        prompt = _required_string(raw_test, "prompt", location)
        target = _required(raw_test, "target", location)
        if not isinstance(target, dict):
            _fail("invalid_type", f"{location}.target", "must be a table")
        target_kind = _required_string(target, "kind", f"{location}.target")
        target_agent: str | None
        if target_kind == "root":
            if set(target) != {"kind"}:
                _fail(
                    "invalid_field",
                    f"{location}.target",
                    "root target requires only kind",
                )
            if root_agent is None:
                _fail(
                    "missing_reference",
                    f"{location}.target",
                    "root target requires root.agent with an explicit Tool policy",
                )
            target_agent = None
        elif target_kind == "agent":
            if set(target) != {"kind", "agent"}:
                _fail(
                    "invalid_field",
                    f"{location}.target",
                    "agent target requires exactly kind and agent",
                )
            target_agent = _required_string(target, "agent", f"{location}.target")
            if target_agent not in agent_ids:
                _fail(
                    "missing_reference",
                    f"{location}.target.agent",
                    f"references undeclared agent {target_agent!r}",
                )
            if target_agent == root_agent:
                _fail(
                    "invalid_field",
                    f"{location}.target.agent",
                    "root Agent must use target.kind = 'root'",
                )
        else:
            _fail(
                "invalid_field",
                f"{location}.target.kind",
                "must equal 'root' or 'agent'",
            )
        tool = _required_string(raw_test, "tool", location)
        server = _required_string(raw_test, "server", location)
        tool_name = _required_string(raw_test, "tool_name", location)
        if tool not in tool_ids:
            _fail(
                "missing_reference",
                f"{location}.tool",
                f"references undeclared tool {tool!r}",
            )
        arguments = raw_test.get("arguments", {})
        if not isinstance(arguments, dict):
            _fail("invalid_type", f"{location}.arguments", "must be a table")
        expect = _required(raw_test, "expect", location)
        if not isinstance(expect, dict):
            _fail("invalid_type", f"{location}.expect", "must be a table")
        if set(expect) != {"structured_content"}:
            _fail(
                "invalid_field",
                f"{location}.expect",
                "requires only structured_content",
            )
        structured = _required(expect, "structured_content", f"{location}.expect")
        if not isinstance(structured, dict):
            _fail(
                "invalid_type",
                f"{location}.expect.structured_content",
                "must be a table",
            )
        _validate_bounded_json(arguments, f"{location}.arguments")
        _validate_bounded_json(
            structured, f"{location}.expect.structured_content"
        )
        tests.append(
            CopilotTestCase(
                id=test_id,
                prompt=prompt,
                target_kind=target_kind,
                target_agent=target_agent,
                tool=tool,
                server=server,
                tool_name=tool_name,
                arguments=arguments,
                expected_structured_content=structured,
            )
        )
    return tests


def _validate_bounded_json(value: Any, location: str, *, depth: int = 0) -> None:
    if depth > 4:
        _fail("invalid_type", location, "must not exceed four nested levels")
    if value is None or type(value) in (bool, int, float):
        return
    if isinstance(value, str):
        if len(value) > 2_000:
            _fail("invalid_type", location, "string must not exceed 2000 characters")
        return
    if isinstance(value, list):
        if len(value) > 32:
            _fail("invalid_type", location, "array must not exceed 32 entries")
        for index, item in enumerate(value):
            _validate_bounded_json(item, f"{location}[{index}]", depth=depth + 1)
        return
    if isinstance(value, dict):
        if len(value) > 32:
            _fail("invalid_type", location, "table must not exceed 32 entries")
        for key, item in value.items():
            if not isinstance(key, str) or not key:
                _fail("invalid_type", location, "table keys must be non-empty strings")
            _validate_bounded_json(item, f"{location}.{key}", depth=depth + 1)
        return
    _fail("invalid_type", location, "must contain only JSON-compatible values")


@dataclass(frozen=True)
class _SkillFrontmatter:
    name: str
    description: str
    short_description: str | None


def _skill_frontmatter(path: Path, relative_path: str) -> _SkillFrontmatter:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        _fail("frontmatter_mismatch", relative_path, str(exc))
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        _fail("frontmatter_mismatch", relative_path, "missing YAML frontmatter")
    try:
        end = next(index for index, line in enumerate(lines[1:], 1) if line.strip() == "---")
    except StopIteration:
        _fail("frontmatter_mismatch", relative_path, "unterminated YAML frontmatter")
    fields: dict[str, str] = {}
    metadata: dict[str, str] = {}
    in_metadata = False
    for line in lines[1:end]:
        if not line.strip():
            continue
        if line[0].isspace():
            if not in_metadata or not line.startswith("  ") or line.startswith("   "):
                _fail(
                    "frontmatter_mismatch",
                    relative_path,
                    "metadata fields must use one two-space indentation level",
                )
            key, value = _skill_frontmatter_scalar(
                line[2:], relative_path, "metadata.short-description"
            )
            if key != "short-description" or key in metadata:
                _fail("frontmatter_mismatch", relative_path, "unknown or duplicate metadata field")
            metadata[key] = value
            continue
        in_metadata = False
        if line.startswith(("-", "[", "{", ">", "|")):
            _fail(
                "frontmatter_mismatch",
                relative_path,
                "frontmatter must contain only top-level single-line scalars",
            )
        if line == "metadata:":
            in_metadata = True
            continue
        key, value = _skill_frontmatter_scalar(line, relative_path, "frontmatter")
        if key not in ("name", "description") or key in fields:
            _fail("frontmatter_mismatch", relative_path, "unknown or duplicate field")
        fields[key] = value
    if set(fields) != {"name", "description"}:
        _fail(
            "frontmatter_mismatch",
            relative_path,
            "frontmatter requires name and description exactly once",
        )
    name = fields["name"]
    if len(name) > 64 or any(
        character not in "abcdefghijklmnopqrstuvwxyz0123456789-" for character in name
    ) or name.startswith("-") or name.endswith("-"):
        _fail(
            "frontmatter_mismatch",
            relative_path,
            "name must be 1-64 lowercase letters, digits, or hyphens",
        )
    return _SkillFrontmatter(
        name=name,
        description=fields["description"],
        short_description=metadata.get("short-description"),
    )


def _skill_frontmatter_scalar(
    line: str,
    relative_path: str,
    field: str,
) -> tuple[str, str]:
    if ":" not in line:
        _fail("frontmatter_mismatch", relative_path, "invalid frontmatter field")
    key, raw_value = line.split(":", 1)
    if not key or key.strip() != key:
        _fail("frontmatter_mismatch", relative_path, "invalid frontmatter field")
    raw_value = raw_value.strip()
    if not raw_value or raw_value[0] in ("[", "{", ">", "|", "&", "*"):
        _fail("frontmatter_mismatch", relative_path, f"{field} must be a scalar")
    if raw_value[0] in "\"'":
        if len(raw_value) < 2 or raw_value[-1] != raw_value[0]:
            _fail("frontmatter_mismatch", relative_path, f"{field} has invalid quotes")
        value = raw_value[1:-1]
    else:
        value = raw_value.split(" #", 1)[0].rstrip()
        if value.startswith(("!", "?")):
            _fail("frontmatter_mismatch", relative_path, f"{field} must be plain text")
    if not value.strip():
        _fail("frontmatter_mismatch", relative_path, f"{field} must not be empty")
    return key, value
def _validate_role_references(
    role: dict[str, Any],
    relative_path: str,
    skill_ids: tuple[str, ...],
    tool_ids: tuple[str, ...],
) -> None:
    skills_table = role.get("skills")
    if skills_table is not None:
        if not isinstance(skills_table, dict):
            _fail("invalid_type", f"{relative_path}.skills", "must be a table")
        config = skills_table.get("config")
        if config is not None:
            if not isinstance(config, list):
                _fail("invalid_type", f"{relative_path}.skills.config", "must be an array")
            for index, item in enumerate(config):
                location = f"{relative_path}.skills.config[{index}]"
                if not isinstance(item, dict):
                    _fail("invalid_type", location, "must be a table")
                name = _required_string(item, "name", location)
                enabled = item.get("enabled", True)
                if not isinstance(enabled, bool):
                    _fail("invalid_type", f"{location}.enabled", "must be a boolean")
                if enabled and name not in skill_ids:
                    _fail(
                        "missing_reference",
                        f"{location}.name",
                        f"references undeclared skill {name!r}",
                    )

    mcp_servers = role.get("mcp_servers")
    if mcp_servers is not None:
        _fail(
            "invalid_field",
            f"{relative_path}.mcp_servers",
            "authored Roles must declare Tool policy through plugins; Runtime transport is derived",
        )

    _role_tool_policies(role, relative_path, tool_ids)


def _role_tool_policies(
    role: dict[str, Any], relative_path: str, tool_ids: tuple[str, ...]
) -> dict[str, dict[str, None]]:
    """Return native selected-Plugin MCP policy by declared capability-root ID."""

    policies: dict[str, dict[str, None]] = {}
    plugins = role.get("plugins")
    if plugins is None:
        return policies
    if not isinstance(plugins, dict):
        _fail("invalid_type", f"{relative_path}.plugins", "must be a table")
    for plugin_id, plugin in plugins.items():
        location = f"{relative_path}.plugins.{plugin_id}"
        if plugin_id not in tool_ids:
            _fail(
                "missing_reference",
                location,
                f"references undeclared tool capability root {plugin_id!r}",
            )
        if not isinstance(plugin, dict):
            _fail("invalid_type", location, "must be a table")
        servers = plugin.get("mcp_servers")
        if not isinstance(servers, dict):
            _fail("required_field", f"{location}.mcp_servers", "required table is missing")
        server_policies: dict[str, None] = {}
        for server_id, server in servers.items():
            server_location = f"{location}.mcp_servers.{server_id}"
            if not isinstance(server_id, str) or not server_id:
                _fail("invalid_type", f"{location}.mcp_servers", "keys must be strings")
            if not isinstance(server, dict):
                _fail("invalid_type", server_location, "must be a table")
            unknown = sorted(set(server) - ROLE_MCP_SERVER_POLICY_FIELDS)
            if unknown:
                _fail(
                    "invalid_field",
                    f"{server_location}.{unknown[0]}",
                    "field is not part of Role MCP server policy schema v1",
                )
            for boolean_field in ("enabled", "required"):
                if boolean_field in server and not isinstance(server[boolean_field], bool):
                    _fail(
                        "invalid_type",
                        f"{server_location}.{boolean_field}",
                        "must be a boolean",
                    )
            omitted = server.get("omit_tools_from")
            if "omit_tools_from" in server and omitted != ["direct"]:
                _fail(
                    "invalid_type",
                    f"{server_location}.omit_tools_from",
                    "when supplied, must be exactly [\"direct\"]; omit the field to let Runtime choose direct or deferred exposure",
                )
            server_policies[server_id] = None
        policies[plugin_id] = server_policies
    return policies
