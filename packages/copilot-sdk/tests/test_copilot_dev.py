from __future__ import annotations

import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

from copilot_sdk.app_server_client import AppServerClient
from copilot_sdk.cli import _safe_expected_mcp_statuses, main
from copilot_sdk.dev_profile import (
    OWNER_MARKER,
    CopilotDevError,
    default_tool_build_store_root,
    default_tool_environment_root,
    load_dev_composition,
    prepare_dev_profile,
    validate_workspace,
)
from copilot_sdk.tool_environment import (
    MaterializedCapabilityRoot,
    MaterializedToolComposition,
)


class CopilotDevProfileTests(unittest.TestCase):
    def make_source(self, parent: Path) -> Path:
        root = parent / "source"
        (root / "skills" / "supervisor" / "references").mkdir(parents=True)
        (root / "agents").mkdir()
        (root / "tools" / "native" / ".codex-plugin").mkdir(parents=True)
        (root / "tools" / "native" / "bin").mkdir()
        (root / "skills" / "supervisor" / "SKILL.md").write_text(
            "---\nname: supervisor\ndescription: Coordinate work.\n---\n",
            encoding="utf-8",
        )
        (root / "skills" / "supervisor" / "references" / "guide.md").write_text(
            "complete tree\n", encoding="utf-8"
        )
        (root / "agents" / "worker.toml").write_text(
            'name = "worker"\n[[skills.config]]\nname = "supervisor"\nenabled = true\n'
            '[plugins.native]\nenabled = true\n'
            '[plugins.native.mcp_servers.native]\n'
            'enabled = true\nrequired = true\ndefault_tools_approval_mode = "approve"\n'
            'omit_tools_from = ["direct"]\n',
            encoding="utf-8",
        )
        (root / "tools" / "native" / ".codex-plugin" / "plugin.json").write_text(
            '{"mcpServers":"./.mcp.json"}', encoding="utf-8"
        )
        (root / "tools" / "native" / ".mcp.json").write_text(
            '{"mcpServers":{"native":{"command":"./bin/native-launcher","cwd":".",'
            '"args":[],"env_vars":["OPEN_WEB_CODEX_DATA_DIR"],'
            '"enabled_tools":["transport-default"]}}}',
            encoding="utf-8",
        )
        launcher = root / "tools" / "native" / "bin/native-launcher"
        launcher.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
        launcher.chmod(0o755)
        setup = root / "tools" / "native" / "bin/setup-env"
        setup.write_text(
            "#!/usr/bin/env bash\n"
            "set -euo pipefail\n"
            "mkdir -p \"$OPEN_WEB_CODEX_DATA_DIR/tool-envs/native\"\n"
            "touch \"$OPEN_WEB_CODEX_DATA_DIR/tool-envs/native/setup-invoked\"\n",
            encoding="utf-8",
        )
        setup.chmod(0o755)
        (root / "tools/native/pyproject.toml").write_text(
            '[build-system]\nrequires=["setuptools>=77"]\n'
            'build-backend="setuptools.build_meta"\n'
            '[project]\nname="native-tool"\nversion="0.1.0"\n', encoding="utf-8"
        )
        (root / "tools/native/requirements.lock").write_text(
            "mcp==1.0.0 \\\n    --hash=sha256:" + "a" * 64 + "\n"
            "setuptools==80.9.0 \\\n    --hash=sha256:" + "b" * 64 + "\n",
            encoding="utf-8",
        )
        (root / "tools/native/runtime.toml").write_text(
            "schema_version=1\n"
            "[[dependencies]]\nid='python'\nkind='python-project'\n"
            "manifest='pyproject.toml'\nlock='requirements.lock'\n"
            "[[servers]]\nid='native'\n"
            "entry={kind='python-module',dependency='python',module='server'}\n",
            encoding="utf-8",
        )
        (root / "copilot.toml").write_text(
            'schema_version = 1\nid = "sample"\ndisplay_name = "Sample"\n'
            '[root]\nskill = "supervisor"\ntask_skills = "all"\n'
            '[[skills]]\nid = "supervisor"\npath = "skills/supervisor"\n'
            '[[agents]]\nid = "worker"\nrole = "agents/worker.toml"\n'
            '[[tools]]\nid = "native"\nroot = "tools/native"\n'
            'runtime = "tools/native/runtime.toml"\n',
            encoding="utf-8",
        )
        return root

    def test_warehouse_profiles_preserve_native_direct_tool_availability(self) -> None:
        import tomllib

        repository = Path(__file__).resolve().parents[3]
        expected = {
            "warehouse-network": {
                "data_agent": {"supply_chain_data"},
                "network_agent": {"supply_chain", "map_utils"},
            },
            "warehouse-network-single-agent": {
                "warehouse_single_agent": {"supply_chain_data", "supply_chain", "map_utils"},
            },
        }
        for package, agents in expected.items():
            with self.subTest(package=package):
                composition = load_dev_composition(
                    repository / "copilots" / package,
                    tool_registry_root=repository / "tools",
                )
                prepared = prepare_dev_profile(composition)
                try:
                    for agent, expected_servers in agents.items():
                        role = tomllib.loads(
                            (prepared.profile_root / "agents" / f"{agent}.toml").read_text()
                        )
                        policies = {
                            name: policy
                            for plugin in role["plugins"].values()
                            for name, policy in plugin["mcp_servers"].items()
                        }
                        self.assertEqual(set(policies), expected_servers)
                        for name, policy in policies.items():
                            # Without model search capability, omitting direct leaves
                            # these tools code-mode-only and invisible in function mode.
                            self.assertNotIn("direct", policy.get("omit_tools_from", []))
                            self.assertEqual(
                                policy["default_tools_approval_mode"],
                                "approve" if name == "supply_chain_data" else "prompt",
                            )
                            self.assertNotIn("enabled_tools", policy)
                            self.assertNotIn("disabled_tools", policy)
                finally:
                    prepared.cleanup()

    def test_materializes_author_role_policy_without_tools_or_config(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = Path(directory)
            source = self.make_source(temporary_root)
            profile = temporary_root / "profile"
            prepared = prepare_dev_profile(load_dev_composition(source), profile)

            self.assertTrue((profile / OWNER_MARKER).is_file())
            self.assertEqual(
                (profile / "skills/supervisor/references/guide.md").read_text(),
                "complete tree\n",
            )
            with (profile / "agents/worker.toml").open("rb") as handle:
                runtime_role = __import__("tomllib").load(handle)
            policy = runtime_role["plugins"]["native"]["mcp_servers"]["native"]
            self.assertEqual(policy["omit_tools_from"], ["direct"])
            self.assertNotIn("enabled_tools", policy)
            self.assertTrue(policy["required"])
            self.assertEqual(policy["default_tools_approval_mode"], "approve")
            self.assertNotIn("mcp_servers", runtime_role)
            self.assertFalse((profile / "tools").exists())
            self.assertFalse((profile / "config.toml").exists())
            self.assertFalse(prepared.cleanup_on_exit)

    def test_composition_distinguishes_capability_root_from_mcp_server(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            role = source / "agents/worker.toml"
            role.write_text(
                role.read_text(encoding="utf-8").replace(
                    "mcp_servers.native", "mcp_servers.runtime_native"
                ),
                encoding="utf-8",
            )
            runtime = source / "tools/native/runtime.toml"
            runtime.write_text(
                runtime.read_text(encoding="utf-8").replace("id='native'", "id='runtime_native'"),
                encoding="utf-8",
            )

            composition = load_dev_composition(source)

            self.assertEqual(composition.summary.tool_ids, ("native",))
            self.assertEqual(composition.mcp_server_ids, ("runtime_native",))

    def test_default_tool_cache_survives_skill_only_composition_changes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            cache_home = root / "cache"
            with patch.dict(os.environ, {"XDG_CACHE_HOME": str(cache_home)}):
                before = default_tool_environment_root(load_dev_composition(source))
                skill = source / "skills/supervisor/SKILL.md"
                skill.write_text(skill.read_text(encoding="utf-8") + "Prompt update.\n")
                after = default_tool_environment_root(load_dev_composition(source))

            self.assertEqual(before, after)
            self.assertTrue(before.is_relative_to(cache_home))

    def test_default_build_store_is_shared_across_skill_only_composition_changes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            cache_home = root / "cache"
            with patch.dict(os.environ, {"XDG_CACHE_HOME": str(cache_home)}):
                before = default_tool_build_store_root(load_dev_composition(source))
                skill = source / "skills/supervisor/SKILL.md"
                skill.write_text(skill.read_text(encoding="utf-8") + "Prompt update.\n")
                after = default_tool_build_store_root(load_dev_composition(source))

            self.assertEqual(before, after)
            self.assertTrue(before.is_relative_to(cache_home))
            self.assertEqual(before.name, "copilot-tool-builds")

    def test_source_mcp_transport_is_not_role_projection_truth(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            descriptor = source / "tools/native/.mcp.json"
            descriptor.write_text(
                '{"mcpServers":{"native":{"command":"/bin/echo","cwd":"."}}}',
                encoding="utf-8",
            )

            prepared = prepare_dev_profile(load_dev_composition(source), root / "profile")
            with (prepared.profile_root / "agents/worker.toml").open("rb") as handle:
                role = __import__("tomllib").load(handle)
            self.assertIn("plugins", role)
            self.assertNotIn("mcp_servers", role)

    def test_explicit_profile_rejects_foreign_non_empty_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = Path(directory)
            source = self.make_source(temporary_root)
            profile = temporary_root / "profile"
            profile.mkdir()
            (profile / "foreign.txt").write_text("keep", encoding="utf-8")
            with self.assertRaises(CopilotDevError) as caught:
                prepare_dev_profile(load_dev_composition(source), profile)
            self.assertEqual(caught.exception.code, "ProfileConflict")
            self.assertEqual((profile / "foreign.txt").read_text(), "keep")

    def test_explicit_profile_root_symlink_is_rejected_without_touching_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            external = root / "external"
            external.mkdir()
            sentinel = external / "sentinel.txt"
            sentinel.write_text("keep", encoding="utf-8")
            profile = root / "profile"
            profile.symlink_to(external, target_is_directory=True)

            with self.assertRaises(CopilotDevError) as caught:
                prepare_dev_profile(load_dev_composition(source), profile)

            self.assertEqual(caught.exception.code, "ProfileConflict")
            self.assertEqual(sentinel.read_text(encoding="utf-8"), "keep")
            self.assertTrue(profile.is_symlink())

    def test_owner_marker_symlink_is_rejected_without_touching_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            profile = root / "profile"
            profile.mkdir()
            external = root / "external-marker.json"
            external.write_text("external secret", encoding="utf-8")
            (profile / OWNER_MARKER).symlink_to(external)

            with self.assertRaises(CopilotDevError) as caught:
                prepare_dev_profile(load_dev_composition(source), profile)

            self.assertEqual(caught.exception.code, "ProfileConflict")
            self.assertEqual(external.read_text(encoding="utf-8"), "external secret")
            self.assertTrue((profile / OWNER_MARKER).is_symlink())

    def test_process_root_is_cleaned_when_explicit_profile_validation_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            profile = root / "profile"
            profile.write_text("not a directory", encoding="utf-8")
            process_root = root / "bounded-process"

            def make_process_root(*, prefix: str) -> str:
                self.assertEqual(prefix, "copilot-dev-process-")
                process_root.mkdir()
                return str(process_root)

            with patch(
                "copilot_sdk.dev_profile.tempfile.mkdtemp",
                side_effect=make_process_root,
            ), self.assertRaises(CopilotDevError):
                prepare_dev_profile(load_dev_composition(source), profile)

            self.assertFalse(process_root.exists())
            self.assertEqual(profile.read_text(encoding="utf-8"), "not a directory")

    def test_temporary_profile_cleanup_is_owned_and_bounded(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = self.make_source(Path(directory))
            prepared = prepare_dev_profile(load_dev_composition(source))
            profile = prepared.profile_root
            process_root = prepared.process_root
            self.assertTrue(profile.is_dir())
            self.assertFalse(process_root.is_relative_to(profile))
            prepared.cleanup()
            self.assertFalse(profile.exists())
            self.assertFalse(process_root.exists())

    def test_workspace_must_be_absolute_existing_directory(self) -> None:
        with self.assertRaises(CopilotDevError) as caught:
            validate_workspace(Path("relative"))
        self.assertEqual(caught.exception.code, "WorkspaceInvalid")


class CopilotDevCliTests(unittest.TestCase):
    def make_source(self, parent: Path) -> Path:
        return CopilotDevProfileTests().make_source(parent)

    def make_fake_codex(
        self,
        parent: Path,
        *,
        skill_errors: list[dict[str, str]] | None = None,
        mcp_tools: dict[str, object] | None = None,
        status_delay_seconds: float = 0.0,
    ) -> tuple[Path, Path]:
        executable = parent / "fake-codex"
        transcript = parent / "transcript.json"
        skill_errors = [] if skill_errors is None else skill_errors
        mcp_tools = {"health": {"name": "health"}} if mcp_tools is None else mcp_tools
        executable.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys, time\n"
            f"transcript = {str(transcript)!r}\n"
            "messages = []\n"
            "for line in sys.stdin:\n"
            "    message = json.loads(line)\n"
            "    messages.append(message)\n"
            "    method = message.get('method')\n"
            "    if method == 'initialize': result = {'codexHome': os.environ['CODEX_HOME']}\n"
            f"    elif method == 'skills/list': result = {{'data': [{{'cwd': message['params']['cwds'][0], 'skills': [{{'name': 'supervisor'}}], 'errors': {skill_errors!r}}}]}}\n"
            "    elif method == 'thread/start':\n"
            "        result = {'thread': {'id': 'thread-dev'}}\n"
            f"    elif method == 'mcpServerStatus/list':\n        time.sleep({status_delay_seconds!r})\n        result = {{'data': [{{'name': 'native', 'tools': {mcp_tools!r}, 'authStatus': 'unknown'}}], 'nextCursor': None}}\n"
            "    else:\n"
            "        if 'id' not in message: continue\n"
            "        result = {}\n"
            "    if method == 'mcpServerStatus/list':\n"
            "        with open(transcript, 'w', encoding='utf-8') as handle:\n"
            "            json.dump({'messages': messages, 'mcpTools': result['data'][0]['tools'], 'cwd': os.getcwd(), 'profile': os.environ['CODEX_HOME'], 'home': os.environ['HOME']}, handle)\n"
            "    print(json.dumps({'id': message['id'], 'result': result}), flush=True)\n",
            encoding="utf-8",
        )
        executable.chmod(0o755)
        return executable, transcript

    def invoke(self, *arguments: str) -> tuple[int, str, str]:
        stdout = io.StringIO()
        stderr = io.StringIO()
        def fake_prepare(prepared, *, output_root=None, build_store_root=None):
            self.assertIsNone(output_root)
            self.assertIsNone(build_store_root)
            projection = prepared.process_data / "capability-roots/native"
            projection.mkdir(parents=True, exist_ok=True)
            return MaterializedToolComposition(
                (MaterializedCapabilityRoot("native", projection, ()),),
            )

        with (
            redirect_stdout(stdout),
            redirect_stderr(stderr),
            patch("copilot_sdk.cli.prepare_dev_tool_composition", side_effect=fake_prepare),
        ):
            result = main(list(arguments))
        return result, stdout.getvalue(), stderr.getvalue()

    def test_dev_requests_status_immediately_after_thread_start_without_startup_notification(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            workspace = root / "workspace"
            workspace.mkdir()
            codex, transcript_path = self.make_fake_codex(root)

            result, stdout, stderr = self.invoke(
                "dev",
                str(source),
                "--workspace",
                str(workspace),
                "--codex-bin",
                str(codex),
                "--json",
            )

            self.assertEqual(result, 0, stderr)
            self.assertEqual(stderr, "")
            payload = json.loads(stdout)
            self.assertEqual(payload["state"], "discovery_ready")
            self.assertEqual(payload["roleSpawn"], "not_run")
            self.assertEqual(payload["modelAcceptance"], "not_run")
            self.assertEqual(payload["discovery"]["skills"]["observed"], ["supervisor"])
            self.assertEqual(payload["discovery"]["mcpServers"]["observed"], ["native"])
            self.assertNotIn("path", payload["profile"])

            transcript = json.loads(transcript_path.read_text(encoding="utf-8"))
            messages = transcript["messages"]
            self.assertEqual(
                [message["method"] for message in messages],
                [
                    "initialize",
                    "initialized",
                    "skills/list",
                    "thread/start",
                    "mcpServerStatus/list",
                ],
            )
            self.assertTrue(messages[0]["params"]["capabilities"]["experimentalApi"])
            self.assertEqual(messages[2]["params"]["cwds"], [str(workspace.resolve())])
            self.assertEqual(
                messages[3]["params"]["selectedCapabilityRoots"],
                [
                    {
                        "id": "native",
                        "location": {
                            "type": "environment",
                            "environmentId": "local",
                            "path": messages[3]["params"]["selectedCapabilityRoots"][0]["location"]["path"],
                        },
                    }
                ],
            )
            projection_path = messages[3]["params"]["selectedCapabilityRoots"][0]["location"]["path"]
            self.assertIn("/capability-roots/native", projection_path)
            self.assertNotEqual(projection_path, str((source / "tools/native").resolve()))
            self.assertTrue(messages[3]["params"]["ephemeral"])
            self.assertEqual(messages[3]["params"]["approvalPolicy"], "never")
            self.assertEqual(messages[3]["params"]["sandbox"], "read-only")
            self.assertEqual(
                messages[3]["params"]["environments"],
                [{
                    "environmentId": "local",
                    "cwd": str(workspace.resolve()),
                    "runtimeWorkspaceRoots": [str(workspace.resolve())],
                }],
            )
            self.assertEqual(messages[4]["params"]["threadId"], "thread-dev")
            self.assertEqual(messages[4]["params"]["limit"], 100)
            self.assertEqual(transcript["mcpTools"], {"health": {"name": "health"}})
            self.assertNotEqual(transcript["cwd"], transcript["profile"])
            self.assertNotEqual(transcript["home"], transcript["profile"])
            self.assertFalse(Path(transcript["cwd"]).exists())
            self.assertFalse(Path(transcript["home"]).exists())

    def test_dev_explicit_profile_is_preserved_without_tool_copy(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            workspace = root / "workspace"
            workspace.mkdir()
            profile = root / "profile"
            codex, _ = self.make_fake_codex(root)

            result, stdout, stderr = self.invoke(
                "dev", str(source), "--workspace", str(workspace),
                "--profile", str(profile), "--codex-bin", str(codex), "--json"
            )

            self.assertEqual(result, 0, stderr)
            payload = json.loads(stdout)
            self.assertFalse(payload["profile"]["temporary"])
            self.assertTrue(payload["profile"]["preserved"])
            self.assertEqual(
                sorted(path.name for path in profile.iterdir()),
                [OWNER_MARKER, "agents", "skills"],
            )
            self.assertFalse((profile / "tools").exists())

    def test_dev_does_not_require_tool_owned_setup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            (source / "tools/native/bin/setup-env").unlink()
            workspace = root / "workspace"
            workspace.mkdir()
            codex, transcript = self.make_fake_codex(root)

            result, stdout, stderr = self.invoke(
                "dev", str(source), "--workspace", str(workspace),
                "--codex-bin", str(codex), "--json"
            )

            self.assertEqual(result, 0, stderr)
            self.assertEqual(stderr, "")
            self.assertEqual(json.loads(stdout)["state"], "discovery_ready")
            self.assertTrue(transcript.exists())

    def test_dev_does_not_execute_tool_owned_setup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            setup = source / "tools/native/bin/setup-env"
            secret = "SENTINEL_SETUP_SECRET"
            setup.write_text(
                f"#!/usr/bin/env bash\nprintf '{secret}' >&2\nexit 9\n"
            )
            setup.chmod(0o755)
            workspace = root / "workspace"
            workspace.mkdir()
            codex, transcript = self.make_fake_codex(root)

            result, stdout, stderr = self.invoke(
                "dev", str(source), "--workspace", str(workspace),
                "--codex-bin", str(codex), "--json"
            )

            self.assertEqual(result, 0, stderr)
            self.assertEqual(stderr, "")
            self.assertNotIn(secret, stdout)
            self.assertNotIn(secret, stderr)
            self.assertTrue(transcript.exists())

    def test_dev_rejects_skill_discovery_errors_before_name_matching(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            workspace = root / "workspace"
            workspace.mkdir()
            codex, _ = self.make_fake_codex(
                root,
                skill_errors=[{"path": "broken", "message": "parse failed"}],
            )

            result, stdout, stderr = self.invoke(
                "dev", str(source), "--workspace", str(workspace),
                "--codex-bin", str(codex), "--json"
            )

            self.assertEqual(result, 2)
            self.assertEqual(stderr, "")
            error = json.loads(stdout)["error"]
            self.assertEqual(error["code"], "DiscoveryIncomplete")
            self.assertIn("Skill discovery errors", error["message"])

    def test_dev_does_not_count_mcp_server_with_empty_tools(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            workspace = root / "workspace"
            workspace.mkdir()
            codex, _ = self.make_fake_codex(root, mcp_tools={})

            result, stdout, stderr = self.invoke(
                "dev", str(source), "--workspace", str(workspace),
                "--codex-bin", str(codex), "--json"
            )

            self.assertEqual(result, 2)
            self.assertEqual(stderr, "")
            error = json.loads(stdout)["error"]
            self.assertEqual(error["code"], "DiscoveryIncomplete")
            self.assertIn("observed MCP status: native(unknown)", error["message"])

    def test_dev_status_list_timeout_is_typed_without_notification_wait(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            workspace = root / "workspace"
            workspace.mkdir()
            codex, transcript = self.make_fake_codex(root, status_delay_seconds=0.8)

            launch = AppServerClient.launch

            def launch_with_stable_initialization_timeout(*args, **kwargs):
                kwargs["timeout_seconds"] = 5.0
                return launch(*args, **kwargs)

            with patch(
                "copilot_sdk.cli.AppServerClient.launch",
                side_effect=launch_with_stable_initialization_timeout,
            ):
                result, stdout, stderr = self.invoke(
                    "dev", str(source), "--workspace", str(workspace),
                    "--codex-bin", str(codex), "--timeout-seconds", "0.5", "--json"
                )

            self.assertEqual(result, 2)
            self.assertEqual(stderr, "")
            error = json.loads(stdout)["error"]
            self.assertEqual(error["code"], "RpcFailed")
            self.assertEqual(error["stage"], "app-server")
            self.assertIn("timed out waiting for mcpServerStatus/list", error["message"])
            self.assertFalse(transcript.exists())

    def test_safe_mcp_status_projection_uses_only_official_bounded_fields(self) -> None:
        projection = _safe_expected_mcp_statuses(
            [
                {
                    "name": "native",
                    "authStatus": "unknown",
                    "tools": {},
                    "serverInfo": {"description": "do not expose"},
                    "error": {"code": "secret", "message": "do not expose"},
                    "status": "failed",
                    "health": "bad",
                },
                {"name": "unrelated", "authStatus": "unsupported"},
            ],
            ["native"],
        )

        self.assertEqual(projection, [{"name": "native", "authStatus": "unknown"}])

    def test_dev_json_reports_absolute_workspace_failure_without_launch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            result, stdout, stderr = self.invoke(
                "dev", str(source), "--workspace", "relative", "--json"
            )
            self.assertEqual(result, 2)
            self.assertEqual(stderr, "")
            error = json.loads(stdout)["error"]
            self.assertEqual(error["code"], "WorkspaceInvalid")
            self.assertEqual(error["stage"], "workspace")

    def test_dev_rejects_timeout_outside_bounded_window(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = self.make_source(root)
            workspace = root / "workspace"
            workspace.mkdir()

            for value in ("0", "301"):
                result, stdout, stderr = self.invoke(
                    "dev",
                    str(source),
                    "--workspace",
                    str(workspace),
                    "--timeout-seconds",
                    value,
                    "--json",
                )
                self.assertEqual(result, 2)
                self.assertEqual(stderr, "")
                self.assertEqual(json.loads(stdout)["error"]["code"], "InvalidArgument")


if __name__ == "__main__":
    unittest.main()
