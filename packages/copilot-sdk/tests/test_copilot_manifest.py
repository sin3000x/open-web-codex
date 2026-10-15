from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from copilot_sdk.copilot_manifest import (
    CopilotPackageError,
    load_copilot_test_cases,
    validate_copilot_package,
)


class CopilotManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        (self.root / "skills" / "supervisor").mkdir(parents=True)
        (self.root / "skills" / "worker").mkdir(parents=True)
        (self.root / "agents").mkdir()
        (self.root / "tools" / "routes").mkdir(parents=True)
        (self.root / "skills" / "supervisor" / "SKILL.md").write_text(
            "---\nname: supervisor\ndescription: Coordinate work.\nmetadata:\n  short-description: Coordinate tasks.\n---\n# Supervisor\n",
            encoding="utf-8",
        )
        (self.root / "skills" / "worker" / "SKILL.md").write_text(
            "---\nname: worker\ndescription: Perform work.\nmetadata:\n  short-description: Perform focused work.\n---\n# Worker\n",
            encoding="utf-8",
        )
        (self.root / "agents" / "planner.toml").write_text(
            "name = \"planner\"\n\n"
            "[skills]\nconfig = [{ name = \"worker\", enabled = true }]\n\n"
            "[plugins.routes]\nenabled = true\n"
            "[plugins.routes.mcp_servers.routing_api]\n"
            "required = true\n"
            "omit_tools_from = [\"direct\"]\n",
            encoding="utf-8",
        )
        (self.root / "tools/routes/pyproject.toml").write_text(
            '[build-system]\nrequires=["setuptools>=77"]\n'
            'build-backend="setuptools.build_meta"\n'
            '[project]\nname="routes"\nversion="0.1.0"\n',
            encoding="utf-8",
        )
        (self.root / "tools/routes/requirements.lock").write_text(
            "mcp==1.0.0 \\\n    --hash=sha256:" + "a" * 64 + "\n"
            "setuptools==80.9.0 \\\n    --hash=sha256:" + "b" * 64 + "\n",
            encoding="utf-8",
        )
        (self.root / "tools/routes/runtime.toml").write_text(
            "schema_version=1\n"
            "[[dependencies]]\nid='python'\nkind='python-project'\n"
            "manifest='pyproject.toml'\nlock='requirements.lock'\n"
            "[[servers]]\nid='routing_api'\n"
            "entry={kind='python-module',dependency='python',module='routes.server'}\n",
            encoding="utf-8",
        )
        self.write_manifest()

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write_manifest(self, replacement: str | None = None) -> None:
        content = replacement or """schema_version = 1
id = "network-copilot"
display_name = "Network Copilot"

[root]
skill = "supervisor"
task_skills = "none"

[[skills]]
id = "supervisor"
path = "skills/supervisor"

[[skills]]
id = "worker"
path = "skills/worker"

[[agents]]
id = "planner"
role = "agents/planner.toml"

[[tools]]
id = "routes"
root = "tools/routes"
runtime = "tools/routes/runtime.toml"
"""
        (self.root / "copilot.toml").write_text(content, encoding="utf-8")

    def assert_code(self, expected: str) -> None:
        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root)
        self.assertEqual(caught.exception.code, expected)

    def add_test(self, *, server: str = "routing_api", tool_name: str = "health") -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        manifest += f'''\n[[tests]]
id = "health"
prompt = "Check health."
target = {{ kind = "agent", agent = "planner" }}
tool = "routes"
server = "{server}"
tool_name = "{tool_name}"
arguments = {{}}
[tests.expect]
structured_content = {{ status = "ok" }}
'''
        self.write_manifest(manifest)

    def test_valid_package_returns_stable_summary(self) -> None:
        first = validate_copilot_package(self.root)
        second = validate_copilot_package(self.root, Path("copilot.toml"))
        self.assertEqual(first, second)
        self.assertEqual(first.id, "network-copilot")
        self.assertEqual(first.skill_ids, ("supervisor", "worker"))
        self.assertEqual(first.agent_ids, ("planner",))
        self.assertEqual(first.tool_ids, ("routes",))
        self.assertRegex(first.composition_descriptor_sha256, r"^[0-9a-f]{64}$")

    def test_accepts_runtime_short_description_metadata(self) -> None:
        summary = validate_copilot_package(self.root)
        self.assertEqual(summary.skill_ids, ("supervisor", "worker"))

    def test_shared_tool_resolves_only_from_explicit_root_registry(self) -> None:
        registry = self.root / "registry"
        package = registry / "shared-routes"
        registry.mkdir()
        (self.root / "tools" / "routes").rename(package)
        (package / "tool.toml").write_text(
            'schema_version = 1\nid = "shared-routes"\nruntime = "runtime.toml"\n',
            encoding="utf-8",
        )
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(
            manifest.replace(
                'id = "routes"\nroot = "tools/routes"\nruntime = "tools/routes/runtime.toml"',
                'id = "routes"\npackage = "shared-routes"',
            )
        )

        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root)
        self.assertEqual(caught.exception.code, "missing_reference")

        summary = validate_copilot_package(self.root, tool_registry_root=registry)
        self.assertEqual(summary.tool_ids, ("routes",))
        self.assertRegex(summary.composition_descriptor_sha256, r"^[0-9a-f]{64}$")

    def test_test_server_is_distinct_from_capability_root(self) -> None:
        self.add_test()

        summary = validate_copilot_package(self.root)
        case = load_copilot_test_cases(self.root)[0]

        self.assertEqual(summary.test_ids, ("health",))
        self.assertEqual(case.tool, "routes")
        self.assertEqual(case.server, "routing_api")
        self.assertEqual(case.target_kind, "agent")
        self.assertEqual(case.target_agent, "planner")

    def test_root_target_requires_explicit_root_agent_and_uses_its_policy(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        manifest = manifest.replace(
            'task_skills = "none"',
            'task_skills = "none"\nagent = "planner"',
        )
        manifest += '''
[[tests]]
id = "health"
prompt = "Check health."
target = { kind = "root" }
tool = "routes"
server = "routing_api"
tool_name = "health"
arguments = {}
[tests.expect]
structured_content = { status = "ok" }
'''
        self.write_manifest(manifest)

        case = load_copilot_test_cases(self.root)[0]

        self.assertEqual(case.target_kind, "root")
        self.assertIsNone(case.target_agent)

    def test_rejects_more_than_sixteen_tests(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        cases = []
        for index in range(17):
            cases.append(
                f'''\n[[tests]]
id = "health-{index}"
prompt = "Check health."
target = {{ kind = "agent", agent = "planner" }}
tool = "routes"
server = "routing_api"
tool_name = "health"
arguments = {{}}
[tests.expect]
structured_content = {{ status = "ok" }}
'''
            )
        self.write_manifest(manifest + "".join(cases))

        self.assert_code("invalid_type")

    def test_workspace_delivery_uses_fixed_envelope_and_embeds_validated_schema(self) -> None:
        schema = self.root / "tools/routes/result.schema.json"
        schema.write_text(
            '{"$schema":"https://json-schema.org/draft/2020-12/schema",'
            '"type":"object","required":["result"]}',
            encoding="utf-8",
        )
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        manifest += '''
[[deliveries]]
id = "routing-result"
server = "routing_api"
tool = "publish_result"
kind = "workspace_artifact"
schema = "routing_result.v1"
mime_type = "application/json"
display_name = "Routing result"
content_verifier = { kind = "json_schema", schema_path = "tools/routes/result.schema.json" }
'''
        self.write_manifest(manifest)

        summary = validate_copilot_package(self.root)

        self.assertEqual(summary.deliveries[0].server, "routing_api")
        self.assertEqual(summary.deliveries[0].verifier_kind, "json_schema")
        self.assertEqual(summary.deliveries[0].verifier_value["type"], "object")

    def test_delivery_rejects_undeclared_server(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        manifest += '''
[[deliveries]]
id = "routing-card"
server = "missing"
tool = "create_card"
kind = "inline_geojson_map_card"
schema = "map.v3"
mime_type = "application/vnd.open-web-codex.map-card+json"
display_name = "Map"
'''
        self.write_manifest(manifest)
        self.assert_code("missing_reference")

    def test_test_requires_server(self) -> None:
        self.add_test()
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(manifest.replace('server = "routing_api"\n', ""))
        self.assert_code("required_field")

    def test_test_server_must_be_enabled_by_role(self) -> None:
        self.add_test(server="other_api")
        self.assert_code("missing_reference")

    def test_test_tool_name_is_not_limited_by_role_name_allowlist(self) -> None:
        self.add_test(tool_name="delete_all")
        validate_copilot_package(self.root)

    def test_test_server_must_exist_in_capability_root_descriptor(self) -> None:
        self.add_test()
        runtime = self.root / "tools/routes/runtime.toml"
        runtime.write_text(
            runtime.read_text(encoding="utf-8").replace("id='routing_api'", "id='other_api'"),
            encoding="utf-8",
        )
        self.assert_code("missing_reference")

    def test_rejects_absolute_authored_path(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(manifest.replace('path = "skills/worker"', 'path = "/tmp/worker"'))
        self.assert_code("invalid_path")

    def test_rejects_absolute_manifest_inside_source_root(self) -> None:
        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root, self.root / "copilot.toml")
        self.assertEqual(caught.exception.code, "invalid_path")

    def test_rejects_parent_traversal(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(manifest.replace('role = "agents/planner.toml"', 'role = "../planner.toml"'))
        self.assert_code("invalid_path")

    def test_rejects_manifest_escape(self) -> None:
        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root, Path("../copilot.toml"))
        self.assertEqual(caught.exception.code, "invalid_path")

    def test_rejects_symlink_component(self) -> None:
        actual = self.root / "actual"
        actual.mkdir()
        (actual / "SKILL.md").write_text("---\nname: worker\n---\n", encoding="utf-8")
        skill = self.root / "skills" / "worker"
        skill.rename(self.root / "unused-worker")
        skill.symlink_to(actual, target_is_directory=True)
        self.assert_code("unsafe_symlink")

    def test_rejects_duplicate_ids(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(manifest.replace('id = "worker"', 'id = "supervisor"'))
        self.assert_code("duplicate_id")

    def test_requires_root(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(
            manifest.replace(
                '[root]\nskill = "supervisor"\ntask_skills = "none"\n\n', ""
            )
        )
        self.assert_code("required_field")

    def test_requires_a_valid_root_task_skill_policy(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(
            manifest.replace('task_skills = "none"', 'task_skills = "some"')
        )
        self.assert_code("invalid_field")

    def test_rejects_skill_frontmatter_mismatch(self) -> None:
        (self.root / "skills" / "worker" / "SKILL.md").write_text(
            "---\nname: another-worker\ndescription: Perform work.\n---\n", encoding="utf-8"
        )
        self.assert_code("frontmatter_mismatch")

    def test_rejects_invalid_skill_description(self) -> None:
        (self.root / "skills" / "worker" / "SKILL.md").write_text(
            "---\nname: worker\ndescription: |\n  multiline\n---\n", encoding="utf-8"
        )
        self.assert_code("frontmatter_mismatch")

    def test_rejects_invalid_skill_yaml_subset(self) -> None:
        (self.root / "skills" / "worker" / "SKILL.md").write_text(
            "---\nname: worker\ndescription: Perform work.\nunknown: value\n---\n",
            encoding="utf-8",
        )
        self.assert_code("frontmatter_mismatch")

    def test_rejects_invalid_skill_name_rule(self) -> None:
        (self.root / "skills" / "worker" / "SKILL.md").write_text(
            "---\nname: Worker_Name\ndescription: Perform work.\n---\n", encoding="utf-8"
        )
        self.assert_code("frontmatter_mismatch")

    def test_rejects_agent_role_name_mismatch(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(role.read_text(encoding="utf-8").replace('name = "planner"', 'name = "other"'), encoding="utf-8")
        self.assert_code("missing_reference")

    def test_rejects_dangling_role_skill(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(role.read_text(encoding="utf-8").replace("worker", "ghost"), encoding="utf-8")
        self.assert_code("missing_reference")

    def test_rejects_dangling_role_tool(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(role.read_text(encoding="utf-8").replace("routes", "ghost"), encoding="utf-8")
        self.assert_code("missing_reference")

    def test_rejects_authored_role_transport(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(
            'name = "planner"\n[mcp_servers.routes]\ncommand = "./launcher"\n',
            encoding="utf-8",
        )
        self.assert_code("invalid_field")

    def test_rejects_transport_cwd_in_role_mcp_server_policy(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(
            role.read_text(encoding="utf-8") + 'cwd = "/private/tool-source"\n',
            encoding="utf-8",
        )

        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root)

        self.assertEqual(caught.exception.code, "invalid_field")
        self.assertEqual(
            caught.exception.relative_path,
            "agents/planner.toml.plugins.routes.mcp_servers.routing_api.cwd",
        )

    def test_rejects_non_boolean_required_role_mcp_policy(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(
            role.read_text(encoding="utf-8").replace(
                "required = true", 'required = "yes"'
            ),
            encoding="utf-8",
        )

        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root)

        self.assertEqual(caught.exception.code, "invalid_type")
        self.assertEqual(
            caught.exception.relative_path,
            "agents/planner.toml.plugins.routes.mcp_servers.routing_api.required",
        )

    def test_accepts_runtime_selected_exposure_without_an_omission_policy(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(role.read_text().replace('omit_tools_from = ["direct"]\n', ""))
        validate_copilot_package(self.root)

    def test_rejects_invalid_deferred_role_mcp_policy_surface(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(
            role.read_text(encoding="utf-8").replace(
                'omit_tools_from = ["direct"]',
                'omit_tools_from = ["untrusted"]',
            ),
            encoding="utf-8",
        )

        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root)

        self.assertEqual(caught.exception.code, "invalid_type")
        self.assertEqual(
            caught.exception.relative_path,
            "agents/planner.toml.plugins.routes.mcp_servers.routing_api.omit_tools_from",
        )

    def test_rejects_role_tool_name_allowlist(self) -> None:
        role = self.root / "agents" / "planner.toml"
        role.write_text(
            role.read_text(encoding="utf-8") + 'enabled_tools = ["health"]\n',
            encoding="utf-8",
        )

        with self.assertRaises(CopilotPackageError) as caught:
            validate_copilot_package(self.root)

        self.assertEqual(caught.exception.code, "invalid_field")
        self.assertEqual(
            caught.exception.relative_path,
            "agents/planner.toml.plugins.routes.mcp_servers.routing_api.enabled_tools",
        )

    def test_rejects_missing_runtime_descriptor(self) -> None:
        (self.root / "tools/routes/runtime.toml").unlink()
        self.assert_code("missing_file")

    def test_rejects_missing_runtime_dependency_manifest(self) -> None:
        (self.root / "tools/routes/pyproject.toml").unlink()
        self.assert_code("missing_file")

    def test_rejects_runtime_outside_tool_root(self) -> None:
        manifest = (self.root / "copilot.toml").read_text(encoding="utf-8")
        self.write_manifest(manifest.replace(
            'runtime = "tools/routes/runtime.toml"', 'runtime = "copilot.toml"'
        ))
        self.assert_code("invalid_path")

    def test_rejects_empty_runtime_server_registry(self) -> None:
        runtime = self.root / "tools/routes/runtime.toml"
        runtime.write_text(runtime.read_text(encoding="utf-8").split("[[servers]]", 1)[0])
        self.assert_code("required_field")


if __name__ == "__main__":
    unittest.main()
