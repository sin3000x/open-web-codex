from __future__ import annotations

import json
from pathlib import Path

import pytest
from openpyxl import Workbook
from supply_chain_planner.data import workspace_intake
from supply_chain_planner.data.workspace_intake import discover, inspect, read_unit_rows


def test_workspace_discovery_returns_bounded_relative_descriptors(tmp_path: Path) -> None:
    (tmp_path / "demand.csv").write_text(
        "origin,quantity,date\nJakarta,10,2026-01-01\n",
        encoding="utf-8",
    )
    (tmp_path / "nested").mkdir()
    (tmp_path / "nested" / "facilities.json").write_text(
        json.dumps({"facilities": [{"facility_id": "JKT", "latitude": -6.2}]}),
        encoding="utf-8",
    )
    (tmp_path / "notes.txt").write_text("ignored", encoding="utf-8")
    generated = tmp_path / "outputs/warehouse-network/prepared"
    generated.mkdir(parents=True)
    (generated / "prepared.json").write_text("{}", encoding="utf-8")

    sources = discover(tmp_path)

    assert [source["relative_path"] for source in sources] == [
        "demand.csv",
        "nested/facilities.json",
        "outputs/warehouse-network/prepared/prepared.json",
    ]
    assert sources[:2] == [
        {
            "relative_path": "demand.csv",
            "format": "csv",
            "size": (tmp_path / "demand.csv").stat().st_size,
        },
        {
            "relative_path": "nested/facilities.json",
            "format": "json",
            "size": (tmp_path / "nested" / "facilities.json").stat().st_size,
        },
    ]
    assert sources[2]["kind"] == "prepared_candidate"
    assert sources[2]["candidate"] is True


def test_workspace_discovery_rejects_legacy_excel_formats(tmp_path: Path) -> None:
    (tmp_path / "legacy.xlsm").write_bytes(b"not a supported workbook")

    with pytest.raises(ValueError, match="unsupported_source_format"):
        discover(tmp_path)


def test_exact_workspace_source_validation_rejects_escape_symlink_and_size(
    tmp_path: Path, monkeypatch
) -> None:
    (tmp_path / "valid.csv").write_text("id\n1\n", encoding="utf-8")
    (tmp_path / "empty.csv").touch()
    (tmp_path / "unsupported.txt").write_text("id\n1\n", encoding="utf-8")
    outside = tmp_path.parent / f"{tmp_path.name}-outside.csv"
    outside.write_text("id\n2\n", encoding="utf-8")
    (tmp_path / "linked.csv").symlink_to(outside)
    (tmp_path / "linked-dir").symlink_to(tmp_path.parent, target_is_directory=True)

    for relative_path, expected in [
        ("", "relative_path_required"),
        (".", "invalid_workspace_relative_path"),
        ("../outside.csv", "invalid_workspace_relative_path"),
        (str((tmp_path / "valid.csv").resolve()), "invalid_workspace_relative_path"),
        ("linked.csv", "symlink_rejected"),
        (f"linked-dir/{outside.name}", "symlink_rejected"),
        ("unsupported.txt", "unsupported_source_format"),
        ("empty.csv", "source_size_limit"),
    ]:
        with pytest.raises(ValueError, match=expected):
            inspect(tmp_path, relative_path)

    monkeypatch.setattr(workspace_intake, "MAX_BYTES", 4)
    with pytest.raises(ValueError, match="source_size_limit"):
        inspect(tmp_path, "valid.csv")


def test_prepared_candidate_discovery_rejects_symlinked_candidates(tmp_path: Path) -> None:
    prepared = tmp_path / "outputs/warehouse-network/prepared"
    prepared.mkdir(parents=True)
    outside = tmp_path.parent / f"{tmp_path.name}-prepared.json"
    outside.write_text("{}", encoding="utf-8")
    (prepared / "linked.json").symlink_to(outside)

    with pytest.raises(ValueError, match="workspace_source_symlink_rejected"):
        discover(tmp_path)


def test_source_structure_limits_reject_instead_of_truncating(tmp_path: Path, monkeypatch) -> None:
    (tmp_path / "rows.csv").write_text("id\n1\n2\n", encoding="utf-8")
    monkeypatch.setattr(workspace_intake, "MAX_NORMALIZE_ROWS", 1)
    with pytest.raises(ValueError, match="row_limit"):
        inspect(tmp_path, "rows.csv")

    workbook = Workbook()
    workbook.active.append(["first", "second"])
    workbook.save(tmp_path / "columns.xlsx")
    monkeypatch.setattr(workspace_intake, "MAX_XLSX_COLUMNS", 1)
    with pytest.raises(ValueError, match="column_limit"):
        inspect(tmp_path, "columns.xlsx")


def test_csv_and_json_profiles_report_full_column_statistics(tmp_path: Path) -> None:
    csv_path = tmp_path / "demand.csv"
    csv_path.write_text(
        "origin;qty;kind\n"
        + "\n".join(
            f"city-{index};{index};{'center' if index % 5 == 0 else ''}" for index in range(25)
        )
        + "\n",
        encoding="utf-8",
    )
    json_path = tmp_path / "nested.json"
    json_path.write_text(
        json.dumps(
            {
                "orders": [
                    {"origin": f"city-{index}", "quantity": index}
                    if index != 24
                    else {"origin": "city-24"}
                    for index in range(25)
                ]
            }
        ),
        encoding="utf-8",
    )

    sources = discover(tmp_path)
    profiles = {
        source["relative_path"]: inspect(tmp_path, source["relative_path"]) for source in sources
    }

    assert profiles["demand.csv"]["structure"]["delimiter"] == ";"
    assert profiles["demand.csv"]["structure"]["columns"] == ["origin", "qty", "kind"]
    csv_structure = profiles["demand.csv"]["structure"]
    assert csv_structure["record_count"] == 25
    assert csv_structure["record_count_exact"] is True
    assert "preview" not in csv_structure
    origin, qty, kind = csv_structure["fields"]
    assert origin["value_types"] == {"string": 25}
    assert origin["distinct_count"] == 25
    assert origin["distinct_count_exact"] is True
    assert "value_counts" not in origin
    assert origin["representative_values"] == ["city-0", "city-1", "city-2"]
    assert "representative_values" not in kind
    assert (qty["min"], qty["max"]) == (0, 24)
    assert qty["null_count"] == 0
    # Values beyond the old 20-row head sample are counted, not sampled.
    assert kind["null_count"] == 20
    assert kind["value_counts"] == [{"value": "center", "count": 5}]
    assert "min" not in kind
    json_array = profiles["nested.json"]["structure"]["arrays"][0]
    assert json_array["path"] == "$.orders"
    assert json_array["record_count"] == 25
    assert json_array["record_count_exact"] is True
    assert "preview" not in json_array
    quantity = next(field for field in json_array["fields"] if field["name"] == "quantity")
    assert quantity["null_count"] == 1
    assert quantity["value_types"] == {"integer": 24}
    assert (quantity["min"], quantity["max"]) == (0, 23)


def test_field_statistics_bound_distinct_tracking(tmp_path: Path) -> None:
    (tmp_path / "wide.csv").write_text(
        "code\n" + "\n".join(f"C-{index}" for index in range(300)) + "\n",
        encoding="utf-8",
    )
    (field,) = inspect(tmp_path, "wide.csv")["structure"]["fields"]

    assert field["value_types"] == {"string": 300}
    assert field["distinct_count"] == 256
    assert field["distinct_count_exact"] is False
    assert "value_counts" not in field
    assert len(field["representative_values"]) == 3


def test_read_unit_rows_supports_explicit_json_array_locator(tmp_path: Path) -> None:
    (tmp_path / "administrative-areas.json").write_text(
        json.dumps({"rows": [{"city_id": "IDN-CITY-001", "city_name": "Jakarta"}]}),
        encoding="utf-8",
    )
    source = discover(tmp_path)[0]

    assert read_unit_rows(
        tmp_path,
        source["relative_path"],
        "$.rows",
        locator={"array_prefix": "rows"},
    ) == [
        {"city_id": "IDN-CITY-001", "city_name": "Jakarta"}
    ]


def test_xlsx_profile_preserves_multiple_sheets_and_bounded_rows(tmp_path: Path) -> None:
    workbook = Workbook()
    locations = workbook.active
    locations.title = "需求点"
    locations.append(["Demand Location ID", "Name", "Region"])
    locations.append(["D1", "Jakarta", "ID-JK"])
    facilities = workbook.create_sheet("仓库")
    facilities.append(["Facility ID", "Name", "Existing or Candidate"])
    facilities.append(["F1", "Existing", "existing"])
    workbook.save(tmp_path / "network.xlsx")

    source = discover(tmp_path)[0]
    profile = inspect(tmp_path, source["relative_path"])

    assert source["format"] == "xlsx"
    assert [sheet["sheet"] for sheet in profile["structure"]["sheets"]] == ["需求点", "仓库"]
    assert profile["structure"]["sheets"][0]["columns"] == [
        "Demand Location ID",
        "Name",
        "Region",
    ]
    warehouse_sheet = profile["structure"]["sheets"][1]
    assert "preview" not in warehouse_sheet
    # A single row is not a repeating category, so its value is a sample, not a count.
    assert warehouse_sheet["fields"][2]["representative_values"] == ["existing"]
    assert "value_counts" not in warehouse_sheet["fields"][2]
    assert warehouse_sheet["formula_cell_count"] == 0
    assert warehouse_sheet["record_count"] == 1
    assert warehouse_sheet["record_count_exact"] is True


def test_json_profile_exposes_every_nested_array_as_an_exact_source_unit(tmp_path: Path) -> None:
    (tmp_path / "nested.json").write_text(
        json.dumps(
            {
                "payload": {
                    "cities": [{"city_code": "ID-1", "city_name": "Balikpapan"}],
                    "meta": ["one", "two"],
                    "records": [{"tags": ["a", "b"]}],
                },
                "warehouseRows": [{"warehouse_id": "WH-1"}],
            }
        ),
        encoding="utf-8",
    )

    structure = inspect(tmp_path, "nested.json")["structure"]
    arrays = {array["unit_ref"]: array for array in structure["arrays"]}

    assert set(arrays) == {
        "$.payload.cities",
        "$.payload.meta",
        "$.payload.records",
        "$.payload.records[*].tags",
        "$.warehouseRows",
    }
    cities = arrays["$.payload.cities"]
    assert cities["path"] == "$.payload.cities"
    assert cities["record_count"] == 1
    assert arrays["$.payload.meta"]["non_object_item_count"] == 2
    assert {field["name"] for field in cities["fields"]} == {"city_code", "city_name"}
