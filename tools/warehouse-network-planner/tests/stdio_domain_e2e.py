"""Real stdio closure for the composable warehouse-network domain tools.

This is a test harness, not a production workflow. It proves a close-only S3
before an S2 facility optimization while reusing the same exact Resources.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
from pathlib import Path

import pytest
from _network_fixtures import (
    indonesia_current_assignments,
    indonesia_network_fixture,
    indonesia_provided_route_facts,
    indonesia_route_quotes,
)
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
from pydantic import AnyUrl

ROOT = Path(__file__).resolve().parents[1]
SANDBOX_META = "codex/sandbox-state-meta"
BEKASI_ID = "WH-CROSS_DOCKING-BEKASI"
EXPECTED_OPENED = ["WH-CANDIDATE-KENDARI", "WH-CANDIDATE-MANADO"]


def _environment(state_root: Path) -> dict[str, str]:
    environment = dict(os.environ)
    environment.update(
        {
            "CODEX_HOME": str(state_root / "codex-home"),
            "OPEN_WEB_CODEX_DATA_DIR": str(state_root / "runtime"),
            "PYTHONDONTWRITEBYTECODE": "1",
        }
    )
    return environment


def _meta(workspace: Path) -> dict[str, object]:
    return {SANDBOX_META: {"sandboxCwd": workspace.as_uri()}}


async def _call(
    session: ClientSession,
    name: str,
    arguments: dict[str, object],
    workspace: Path,
):
    result = await asyncio.wait_for(
        session.call_tool(name, arguments, meta=_meta(workspace)),
        timeout=60,
    )
    assert result.isError is not True, result.content
    assert result.structuredContent is not None
    return result


async def _read_resource(
    session: ClientSession,
    ref: dict[str, object],
) -> dict[str, object]:
    result = await asyncio.wait_for(
        session.read_resource(AnyUrl(str(ref["uri"]))),
        timeout=30,
    )
    assert len(result.contents) == 1
    return json.loads(result.contents[0].text)


def _write_sources(workspace: Path) -> list[str]:
    fixture = indonesia_network_fixture()
    facts = {
        (item.origin_id, item.destination_id, item.layer): item
        for item in indonesia_provided_route_facts()
    }
    sources = {
        "demand.json": [item.model_dump(mode="json") for item in fixture.demand],
        "existing.json": [
            item.model_dump(mode="json") for item in fixture.warehouses if item.is_existing
        ],
        "candidates.json": [
            item.model_dump(mode="json") for item in fixture.warehouses if not item.is_existing
        ],
        "assignments.json": [
            item.model_dump(mode="json") for item in indonesia_current_assignments()
        ],
        "quotes.json": [
            {
                **item.model_dump(mode="json"),
                **facts[(item.origin_id, item.destination_id, item.layer)].model_dump(
                    mode="json",
                    include={
                        "destination_name",
                        "distance_km",
                        "duration_hours",
                    },
                ),
                "method": facts[(item.origin_id, item.destination_id, item.layer)].source_method,
            }
            for item in indonesia_route_quotes()
        ],
    }
    for relative_path, rows in sources.items():
        (workspace / relative_path).write_text(
            json.dumps(rows, ensure_ascii=False),
            encoding="utf-8",
        )
    return list(sources)


def _source_selections() -> list[dict[str, object]]:
    return [
        {"relative_path": "demand.json", "unit_ref": "$", "role": "demand"},
        {"relative_path": "existing.json", "unit_ref": "$", "role": "existing_warehouse"},
        {"relative_path": "candidates.json", "unit_ref": "$", "role": "candidate_warehouse"},
        {"relative_path": "assignments.json", "unit_ref": "$", "role": "current_assignment"},
        {"relative_path": "quotes.json", "unit_ref": "$", "role": "route_quote"},
    ]


async def _prepare_normalized_resource(
    workspace: Path,
    environment: dict[str, str],
) -> tuple[str, list[str]]:
    relative_paths = _write_sources(workspace)
    trace: list[str] = []
    parameters = StdioServerParameters(
        command=sys.executable,
        args=["-m", "supply_chain_planner.data.server"],
        cwd=str(workspace),
        env=environment,
    )
    async with stdio_client(parameters) as streams:
        async with ClientSession(*streams) as session:
            await asyncio.wait_for(session.initialize(), timeout=30)
            inventory = {tool.name for tool in (await session.list_tools()).tools}
            assert inventory == {
                "discover_workspace_sources",
                "inspect_workspace_sources",
                "prepare_network_input",
                "prepare_network_geography",
                "query_prepared_network_input",
                "resolve_prepared_network_input",
            }
            assert (await asyncio.wait_for(session.list_resources(), timeout=30)).resources == []
            assert (
                await asyncio.wait_for(session.list_resource_templates(), timeout=30)
            ).resourceTemplates == []
            trace.append("inspect_workspace_sources")
            profile = await _call(
                session,
                "inspect_workspace_sources",
                {
                    "relative_paths": relative_paths,
                    "country_code": "ID",
                },
                workspace,
            )
            trace.append("prepare_network_input")
            normalized = await _call(
                session,
                "prepare_network_input",
                {
                    "inspection_identity": profile.structuredContent["inspection_identity"],
                    "inspected_relative_paths": profile.structuredContent[
                        "inspected_relative_paths"
                    ],
                    "source_selections": _source_selections(),
                    "country_code": "ID",
                    "output_relative_path": "outputs/warehouse-network/prepared/domain.json",
                },
                workspace,
            )
            prepared_path = normalized.structuredContent["prepared_input_relative_path"]
            payload = json.loads((workspace / prepared_path).read_text(encoding="utf-8"))
            assert payload["state"] == "ready"
            assert len(payload["demand_cities"]) == 50
            assert len(payload["warehouses"]) == 23
            assert len(payload["current_assignments"]) == 50
            assert len(payload["route_quotes"]) == 580
            assert len(payload["provided_route_facts"]) == 580
            assert normalized.structuredContent["input_identity"]["content_sha256"]
            return prepared_path, trace


def _cost_policy() -> dict[str, object]:
    return {
        "rules": [
            {
                "layer": layer,
                "currency": "IDR",
                "fixed_cost_per_demand_unit": 40_000,
                "cost_per_km_per_demand_unit": 1_500,
            }
            for layer in ("last_mile", "linehaul")
        ]
    }


async def _run_network_s3_then_s2(
    workspace: Path,
    environment: dict[str, str],
    prepared_path: str,
) -> tuple[list[str], list[str]]:
    parameters = StdioServerParameters(
        command=sys.executable,
        args=["-m", "supply_chain_planner.network.server"],
        cwd=str(workspace),
        env=environment,
    )
    async with stdio_client(parameters) as streams:
        async with ClientSession(*streams) as session:
            await asyncio.wait_for(session.initialize(), timeout=30)
            inventory = {tool.name for tool in (await session.list_tools()).tools}
            assert inventory == {
                "create_navigation_matrix_request",
                "prepare_route_matrix",
                "import_navigation_matrix",
                "plan_cost_matrix",
                "prepare_network_comparison_map",
                "prepare_network_coverage_map",
                "prepare_network_distribution_map",
                "evaluate_network_baseline",
                "assess_facility_change",
                "solve_p_median",
                "compare_network_scenarios",
                "publish_network_planning_report",
            }
            prepared = json.loads((workspace / prepared_path).read_text(encoding="utf-8"))
            existing_ids = sorted(
                item["warehouse_id"] for item in prepared["warehouses"] if item["is_existing"]
            )
            common_trace: list[str] = []
            common_trace.append("prepare_route_matrix:provided")
            provided_result = await _call(
                session,
                "prepare_route_matrix",
                {
                    "prepared_input_relative_path": prepared_path,
                    "route_method": "provided",
                    "warehouse_scope": {"kind": "existing_only"},
                },
                workspace,
            )
            provided_ref = provided_result.structuredContent["resource_ref"]
            provided = await _read_resource(session, provided_ref)
            assert len(provided["rows"]) == 556
            assert provided["stats"]["missing_pair_count"] == 0

            common_trace.append("prepare_route_matrix:haversine")
            routes_result = await _call(
                session,
                "prepare_route_matrix",
                {
                    "prepared_input_relative_path": prepared_path,
                    "route_method": "haversine",
                    "warehouse_scope": {"kind": "all_warehouses"},
                    "detour_coefficient": 1.2,
                    "average_speed_kph": 42,
                },
                workspace,
            )
            routes_ref = routes_result.structuredContent["resource_ref"]
            routes = await _read_resource(session, routes_ref)
            assert len(routes["rows"]) == 1168
            assert routes["stats"]["computed_pair_count"] == 1168
            assert routes["stats"]["missing_pair_count"] == 0

            common_trace.append("plan_cost_matrix")
            costs_result = await _call(
                session,
                "plan_cost_matrix",
                {
                    "prepared_input_relative_path": prepared_path,
                    "warehouse_scope": {"kind": "all_warehouses"},
                    "cost_policy": {
                        "kind": "explicit",
                        "rules": _cost_policy()["rules"],
                    },
                    "route_matrix_ref": routes_ref,
                },
                workspace,
            )
            costs_ref = costs_result.structuredContent["resource_ref"]
            costs = await _read_resource(session, costs_ref)
            assert len(costs["rows"]) == 1168
            assert costs["missing_routes"] == []

            baseline_costs_result = await _call(
                session,
                "plan_cost_matrix",
                {
                    "prepared_input_relative_path": prepared_path,
                    "warehouse_scope": {"kind": "existing_only"},
                    "cost_policy": {
                        "kind": "explicit",
                        "rules": _cost_policy()["rules"],
                    },
                    "route_matrix_ref": provided_ref,
                },
                workspace,
            )
            baseline_costs_ref = baseline_costs_result.structuredContent["resource_ref"]

            common_trace.append("evaluate_network_baseline")
            baseline_result = await _call(
                session,
                "evaluate_network_baseline",
                {
                    "prepared_input_relative_path": prepared_path,
                    "route_matrix_ref": provided_ref,
                    "cost_matrix_ref": baseline_costs_ref,
                    "objective": "min_cost",
                    "service_targets": [6, 12, 18],
                    "coverage_mode": "actual_current",
                },
                workspace,
            )
            baseline_ref = baseline_result.structuredContent["resource_ref"]
            baseline = await _read_resource(session, baseline_ref)
            assert baseline["label"] == "actual_current"
            assert baseline["active_warehouse_ids"] == existing_ids
            assert len(baseline["assignment"]["rows"]) == 50
            assert (
                baseline_result.structuredContent["coverage_metrics"][0]["city_coverage_rate"] >= 0
            )
            assert (
                baseline_result.structuredContent["coverage_metrics"][0][
                    "demand_weighted_coverage_rate"
                ]
                >= 0
            )

            s3_trace: list[str] = []
            s3_trace.append("assess_facility_change")
            assessment_result = await _call(
                session,
                "assess_facility_change",
                {
                    "prepared_input_relative_path": prepared_path,
                    "route_matrix_ref": provided_ref,
                    "cost_matrix_ref": baseline_costs_ref,
                    "before_ref": baseline_ref,
                    "scenario": {
                        "add_warehouse_ids": [],
                        "remove_warehouse_ids": [BEKASI_ID],
                        "relocations": [],
                        "objective": "min_cost",
                        "service_targets": [12],
                    },
                },
                workspace,
            )
            assert assessment_result.structuredContent["active_warehouse_count"] == 10
            scenario_ref = assessment_result.structuredContent["scenario_ref"]
            scenario = await _read_resource(session, scenario_ref)
            assert scenario["active_warehouse_ids"] == [
                item for item in existing_ids if item != BEKASI_ID
            ]
            assert scenario["warehouse_changes"] == {
                "added": [],
                "removed": [BEKASI_ID],
            }
            s3_comparison = await _read_resource(
                session,
                assessment_result.structuredContent["plan_comparison_ref"],
            )
            assert s3_comparison["comparison"]["selected_warehouse_ids"] == []
            assert s3_comparison["comparison"]["removed_warehouse_ids"] == [BEKASI_ID]
            assert len(s3_comparison["comparison"]["affected_city_ids"]) == 50
            assert len(s3_comparison["comparison"]["reassigned_city_ids"]) == 50
            assert set(s3_trace).isdisjoint(
                {
                    "solve_p_median",
                    "prepare_network_comparison_map",
                    "publish_network_planning_report",
                }
            )

            s2_trace: list[str] = []
            s2_trace.append("solve_p_median")
            facility_result = await _call(
                session,
                "solve_p_median",
                {
                    "prepared_input_relative_path": prepared_path,
                    "route_matrix_ref": routes_ref,
                    "cost_matrix_ref": costs_ref,
                    "opening_policy": {"kind": "exact", "number_to_open": 2},
                    "existing_warehouse_policy": {"mode": "keep_all_existing"},
                    "service_targets": [6, 12, 18],
                    "time_limit_seconds": 30,
                },
                workspace,
            )
            facility_ref = facility_result.structuredContent["resource_ref"]
            facility = await _read_resource(session, facility_ref)
            assert facility["status"] == "optimal"
            assert facility["selected_number_to_open"] == 2
            assert facility["minimum_number_to_open_proven"] is False
            assert [stage["kind"] for stage in facility["solver_stages"]] == ["minimum_cost"]
            assert facility["opened_candidate_ids"] == EXPECTED_OPENED
            assert facility["closed_existing_ids"] == []
            assert len(facility["active_warehouse_ids"]) == 13
            assert facility["cost"]["complete"] is True
            assert [item["target_hours"] for item in facility["service"]] == [6, 12, 18]

            s2_trace.append("compare_network_scenarios")
            s2_comparison_result = await _call(
                session,
                "compare_network_scenarios",
                {
                    "prepared_input_relative_path": prepared_path,
                    "before_ref": baseline_ref,
                    "after_ref": facility_ref,
                    "service_targets": [6, 12, 18],
                },
                workspace,
            )
            comparison_ref = s2_comparison_result.structuredContent["resource_ref"]
            comparison = await _read_resource(session, comparison_ref)
            assert comparison["comparison"]["selected_warehouse_ids"] == EXPECTED_OPENED
            assert comparison["comparison"]["removed_warehouse_ids"] == []

            final_refs = {
                "plan_comparison_ref": comparison_ref,
            }
            s2_trace.append("prepare_network_comparison_map")
            inline_map_result = await _call(
                session,
                "prepare_network_comparison_map",
                final_refs,
                workspace,
            )
            inline_map = await _read_resource(
                session,
                inline_map_result.structuredContent["data_ref"],
            )
            demand_features = [
                feature
                for feature in inline_map["features"]
                if feature["properties"]["kind"] == "demand"
            ]
            assert demand_features
            assert all(
                feature["properties"]["before_duration_hours"] is not None
                for feature in demand_features
            )
            assert all(
                feature["properties"]["after_duration_hours"] is not None
                for feature in demand_features
            )
            assert "layers" not in inline_map
            assert "extensions" not in inline_map
            s2_trace.append("publish_network_planning_report")
            report_result = await _call(
                session,
                "publish_network_planning_report",
                {
                    "report_input": {
                        "mode": "comparison",
                        **final_refs,
                    },
                    "output_relative_path": (
                        "outputs/warehouse-network/deliverables/sample2-report.md"
                    ),
                },
                workspace,
            )
            assert report_result.structuredContent["artifact"]["schema"] == (
                "network_planning_report_markdown.v2"
            )
            report_markdown = (
                workspace / "outputs/warehouse-network/deliverables/sample2-report.md"
            ).read_text(encoding="utf-8")
            assert "# 仓网规划结果简报" in report_markdown
            assert "## 时效覆盖" in report_markdown
            assert "结构化计算结果" in report_markdown
            assert report_result.content[0].text == (
                "正式简报已生成：[下载中文 Markdown 简报](outputs/warehouse-network/"
                "deliverables/sample2-report.md)"
            )
            return [*common_trace, *s3_trace], [*common_trace, *s2_trace]


async def smoke() -> None:
    with tempfile.TemporaryDirectory(prefix="warehouse-network-domain-e2e-") as directory:
        state_root = Path(directory)
        workspace = state_root / "workspace"
        workspace.mkdir()
        environment = _environment(state_root)
        prepared_path, data_trace = await _prepare_normalized_resource(
            workspace,
            environment,
        )
        assert data_trace == ["inspect_workspace_sources", "prepare_network_input"]
        s3_trace, s2_trace = await _run_network_s3_then_s2(
            workspace,
            environment,
            prepared_path,
        )
        assert s3_trace[-1:] == ["assess_facility_change"]
        assert s2_trace[-3:] == [
            "compare_network_scenarios",
            "prepare_network_comparison_map",
            "publish_network_planning_report",
        ]


def test_stdio_domain_s3_then_s2() -> None:
    if os.environ.get("RUN_REAL_STDIO_DOMAIN_E2E") != "1":
        pytest.skip("set RUN_REAL_STDIO_DOMAIN_E2E=1 to launch the real stdio closure")
    asyncio.run(asyncio.wait_for(smoke(), timeout=180))


if __name__ == "__main__":
    asyncio.run(asyncio.wait_for(smoke(), timeout=180))
