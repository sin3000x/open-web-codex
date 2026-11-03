"""Bounded Workspace source discovery and evidence profiling.

Inspection streams every record of each source unit once and reduces each
field to bounded statistics (types, nulls, distinct values, numeric range and
low-cardinality value counts) plus a few representative values.  Raw rows are
never returned to the model; complete rows are read again only by the
deterministic normalizer after the mapping is confirmed.
"""

from __future__ import annotations

import csv
import hashlib
import io
import itertools
import json
import re
import stat
import zipfile
from collections import Counter
from collections.abc import Mapping
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path, PurePosixPath
from typing import Any, BinaryIO
from xml.etree import ElementTree

import ijson
from charset_normalizer import from_bytes
from openpyxl import load_workbook

MAX_FILES = 500
MAX_BYTES = 100 * 1024 * 1024
MAX_SNIFF_LINES = 20
MAX_TRACKED_DISTINCT_VALUES = 256
MAX_VALUE_COUNTS = 12
MAX_REPRESENTATIVE_VALUES = 3
MAX_JSON_DEPTH = 64
MAX_JSON_NODES = 250_000
MAX_JSON_STRING = 64 * 1024
MAX_XLSX_SHEETS = 32
MAX_XLSX_COLUMNS = 256
MAX_NORMALIZE_ROWS = 1_000_000
MAX_INSPECTION_FILES = 64
MAX_INSPECTION_UNITS = 128
MAX_INSPECTION_BYTES = 512 * 1024
EXCLUDED_DIRS = {
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    ".demo-staging",
    "__pycache__",
    "datasets",
}
SUPPORTED_SUFFIXES = {".xlsx", ".csv", ".json"}
DEMO_MANIFEST_SCHEMA = "demo_workspace_sources.v1"
GENERATED_OUTPUT_ROOT = PurePosixPath("outputs/warehouse-network")
PREPARED_OUTPUT_ROOT = GENERATED_OUTPUT_ROOT / "prepared"


def _is_generated_output(relative: Path) -> bool:
    parts = relative.parts
    return len(parts) >= 2 and parts[:2] == GENERATED_OUTPUT_ROOT.parts


def _is_prepared_candidate(relative: Path) -> bool:
    """Return whether a path is a direct prepared-input candidate.

    Generated output is intentionally excluded from normal source discovery.
    Prepared inputs are the one explicitly supported generated surface because
    a later planning turn may reuse them after validating their provenance.
    """

    return (
        len(relative.parts) == len(PREPARED_OUTPUT_ROOT.parts) + 1
        and relative.parts[: len(PREPARED_OUTPUT_ROOT.parts)] == PREPARED_OUTPUT_ROOT.parts
        and relative.suffix.lower() == ".json"
    )


def _iter_files(root: Path) -> list[Path]:
    root = root.resolve(strict=True)
    files: list[Path] = []
    for path in root.rglob("*"):
        relative = path.relative_to(root)
        if any(part in EXCLUDED_DIRS for part in relative.parts) or _is_generated_output(relative):
            continue
        if path.is_symlink():
            if path.suffix.lower() in SUPPORTED_SUFFIXES | {".xls", ".xlsm"}:
                raise ValueError("workspace_source_symlink_rejected")
            continue
        if path.is_file() and path.suffix.lower() in SUPPORTED_SUFFIXES:
            files.append(path)
    files.sort(key=lambda item: item.relative_to(root).as_posix())
    return files


def _validated_source_path(root: Path, relative_path: str) -> Path:
    """Resolve one model-visible Workspace path without following symlinks."""
    canonical_root = root.resolve(strict=True)
    if not canonical_root.is_dir():
        raise ValueError("workspace_root_not_directory")
    if not isinstance(relative_path, str) or not relative_path:
        raise ValueError("workspace_relative_path_required")
    relative = PurePosixPath(relative_path)
    if (
        relative.is_absolute()
        or not relative.parts
        or relative.as_posix() != relative_path
        or any(part in {"", ".", ".."} for part in relative.parts)
        or any(part in EXCLUDED_DIRS for part in relative.parts)
    ):
        raise ValueError("invalid_workspace_relative_path")

    current = canonical_root
    for index, part in enumerate(relative.parts):
        current = current / part
        try:
            mode = current.lstat().st_mode
        except FileNotFoundError as error:
            raise ValueError("workspace_source_not_found") from error
        if stat.S_ISLNK(mode):
            raise ValueError("workspace_source_symlink_rejected")
        if index < len(relative.parts) - 1 and not stat.S_ISDIR(mode):
            raise ValueError("workspace_source_parent_not_directory")
    if not stat.S_ISREG(current.lstat().st_mode):
        raise ValueError("workspace_source_not_regular_file")
    try:
        current.resolve(strict=True).relative_to(canonical_root)
    except ValueError as error:
        raise ValueError("workspace_source_escape_rejected") from error
    if current.suffix.lower() not in SUPPORTED_SUFFIXES:
        raise ValueError("unsupported_source_format")
    size = current.stat().st_size
    if size <= 0 or size > MAX_BYTES:
        raise ValueError("workspace_source_size_limit")
    return current


def source_descriptor(root: Path, relative_path: str) -> dict[str, Any]:
    """Return the stable public descriptor for one validated Workspace file."""
    path = _validated_source_path(root, relative_path)
    return {
        "relative_path": relative_path,
        "format": path.suffix.lower().removeprefix("."),
        "size": path.stat().st_size,
    }


def discover_prepared_candidates(root: Path) -> list[dict[str, Any]]:
    """Return descriptors for the direct prepared-input files only."""
    prepared_root = root / Path(*PREPARED_OUTPUT_ROOT.parts)
    if prepared_root.is_symlink():
        raise ValueError("prepared_output_directory_invalid")
    if not prepared_root.exists():
        return []
    if not prepared_root.is_dir():
        raise ValueError("prepared_output_directory_invalid")
    descriptors: list[dict[str, Any]] = []
    for path in sorted(prepared_root.iterdir(), key=lambda item: item.name):
        relative = path.relative_to(root)
        if not _is_prepared_candidate(relative):
            continue
        if path.is_symlink():
            if path.suffix.lower() == ".json":
                raise ValueError("workspace_source_symlink_rejected")
            continue
        if not path.is_file() or path.suffix.lower() != ".json":
            continue
        descriptor = source_descriptor(root, relative.as_posix())
        descriptor.update({"kind": "prepared_candidate", "candidate": True})
        descriptors.append(descriptor)
    return descriptors


def discover(root: Path) -> list[dict[str, Any]]:
    legacy = [
        path
        for path in root.rglob("*")
        if not any(part in EXCLUDED_DIRS for part in path.relative_to(root).parts)
        and not _is_generated_output(path.relative_to(root))
        and path.is_file()
        and path.suffix.lower() in {".xls", ".xlsm"}
    ]
    if legacy:
        raise ValueError(
            "unsupported_source_format: legacy .xls/.xlsm files must be exported "
            "as .xlsx, .csv or .json"
        )
    files = _iter_files(root)
    candidates = discover_prepared_candidates(root)
    if len(files) + len(candidates) > MAX_FILES:
        raise ValueError("workspace_source_limit_exceeded: more than 500 supported files")
    descriptors = [source_descriptor(root, path.relative_to(root).as_posix()) for path in files]
    descriptors.extend(candidates)
    return sorted(descriptors, key=lambda item: item["relative_path"])


def workspace_source_metadata(root: Path) -> dict[str, Any]:
    """Return verified source classification without trusting a filename alone."""
    manifests = sorted(root.glob("demo-data/*/demo-manifest.json"))
    if not manifests:
        return {"dataClassification": "workspace_data"}
    if len(manifests) != 1 or manifests[0].is_symlink():
        raise ValueError("demo_manifest_conflict")
    manifest_path = manifests[0]
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("demo_manifest_invalid") from error
    if (
        manifest.get("schemaVersion") != DEMO_MANIFEST_SCHEMA
        or manifest.get("dataClassification") != "synthetic_demo"
        or not isinstance(manifest.get("template"), dict)
        or not isinstance(manifest.get("files"), list)
    ):
        raise ValueError("demo_manifest_invalid")
    expected_names: set[str] = set()
    for item in manifest["files"]:
        if not isinstance(item, dict):
            raise ValueError("demo_manifest_invalid")
        name = item.get("displayName")
        expected_size = item.get("bytes")
        expected_digest = item.get("contentSha256")
        if (
            not isinstance(name, str)
            or Path(name).name != name
            or name in expected_names
            or not isinstance(expected_size, int)
            or not isinstance(expected_digest, str)
            or not re.fullmatch(r"[0-9a-f]{64}", expected_digest)
        ):
            raise ValueError("demo_manifest_invalid")
        expected_names.add(name)
        source = manifest_path.parent / name
        if source.is_symlink() or not source.is_file():
            raise ValueError("demo_manifest_source_missing")
        if source.stat().st_size != expected_size or _sha256_file(source) != expected_digest:
            raise ValueError("demo_manifest_source_mismatch")
    actual_names = {
        path.name
        for path in manifest_path.parent.iterdir()
        if path.is_file() and path.name != manifest_path.name
    }
    if actual_names != expected_names:
        raise ValueError("demo_manifest_source_mismatch")
    template = manifest["template"]
    return {
        "dataClassification": "synthetic_demo",
        "demoTemplate": {
            "id": template.get("id"),
            "version": template.get("version"),
            "seed": template.get("seed"),
        },
    }


def _sha256_file(path: Path) -> str:
    """Verify explicit Demo fixture bytes without exposing a source identity."""
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_inspection_identity(root: Path, relative_paths: list[str]) -> tuple[str, int]:
    """Hash sorted exact Workspace paths and each complete regular-file digest."""
    snapshot = source_inspection_snapshot(root, relative_paths)
    return snapshot.content_sha256, snapshot.source_count


@dataclass(frozen=True)
class SourceInspectionSnapshot:
    """One complete read of an inspected path set and its per-file digests."""

    content_sha256: str
    source_count: int
    file_sha256: dict[str, str]


def source_inspection_snapshot(root: Path, relative_paths: list[str]) -> SourceInspectionSnapshot:
    """Return aggregate and per-file identities from one sorted source read."""
    if not relative_paths or len(relative_paths) > MAX_FILES:
        raise ValueError("relative_paths must contain 1-500 Workspace-relative paths")
    if len(set(relative_paths)) != len(relative_paths):
        raise ValueError("relative_paths must not contain duplicates")
    digest = hashlib.sha256()
    file_sha256: dict[str, str] = {}
    for relative_path in sorted(relative_paths):
        path = _validated_source_path(root, relative_path)
        content_sha256 = _sha256_file(path)
        file_sha256[relative_path] = content_sha256
        digest.update(relative_path.encode("utf-8"))
        digest.update(b"\0")
        digest.update(content_sha256.encode("ascii"))
        digest.update(b"\n")
    return SourceInspectionSnapshot(
        content_sha256=digest.hexdigest(),
        source_count=len(relative_paths),
        file_sha256=file_sha256,
    )


def source_content_sha256(root: Path, relative_path: str) -> str:
    """Return the complete digest for one validated Workspace source file."""

    return _sha256_file(_validated_source_path(root, relative_path))


def inspect(root: Path, relative_path: str) -> dict[str, Any]:
    path = _validated_source_path(root, relative_path)
    record = source_descriptor(root, relative_path)
    if path.suffix.lower() == ".csv":
        structure = inspect_csv(path)
    elif path.suffix.lower() == ".json":
        structure = inspect_json(path)
    else:
        structure = inspect_xlsx(path)
    return {**record, "structure": structure}


def inspect_csv(path: Path) -> dict[str, Any]:
    raw = _read_prefix(path, 256 * 1024)
    text = decode_text(raw)
    sample = text.splitlines()[: MAX_SNIFF_LINES + 1]
    if not sample:
        return {
            "kind": "table",
            "unit_ref": "table",
            "columns": [],
            "fields": [],
            "record_count": 0,
            "record_count_exact": True,
        }
    try:
        dialect = csv.Sniffer().sniff("\n".join(sample[:10]), delimiters=",;\t|")
        delimiter = dialect.delimiter
    except csv.Error:
        delimiter = ","
    with path.open("rb") as binary:
        text_stream = io.TextIOWrapper(
            binary,
            encoding=detect_encoding(_read_prefix(path, 64 * 1024)),
            errors="strict",
        )
        reader = csv.reader(text_stream, delimiter=delimiter)
        first = next(reader, [])
        if len(first) > MAX_XLSX_COLUMNS:
            raise ValueError("source_column_limit_exceeded")
        header = [cell.strip() for cell in first]
        statistics = _UnitStatistics(header, text=True)
        for row in reader:
            if len(row) > MAX_XLSX_COLUMNS:
                raise ValueError("source_column_limit_exceeded")
            if not any(cell not in (None, "") for cell in row):
                continue
            if statistics.record_count >= MAX_NORMALIZE_ROWS:
                raise ValueError("source_row_limit_exceeded")
            statistics.add_row(row)
    return {
        "kind": "table",
        "unit_ref": "table",
        "delimiter": delimiter,
        "columns": header,
        "fields": statistics.field_profiles(),
        "record_count": statistics.record_count,
        "record_count_exact": True,
    }


def _sample_type(value: Any, *, text: bool = False) -> str:
    if value is None or value == "":
        return "null"
    if text:
        candidate = str(value).strip()
        if not candidate:
            return "null"
        if candidate.lower() in {"true", "false", "yes", "no", "y", "n"}:
            return "boolean"
        try:
            decimal_value = Decimal(candidate.replace(",", ""))
        except (InvalidOperation, ValueError):
            return "string"
        return "integer" if decimal_value == decimal_value.to_integral_value() else "number"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, dict):
        return "object"
    if isinstance(value, list):
        return "array"
    return type(value).__name__


def _bounded_representative(value: Any) -> Any:
    if isinstance(value, str):
        return value[:256]
    if isinstance(value, (int, float, bool)) or value is None:
        return value
    return str(value)[:256]


def _numeric_value(value: Any, value_type: str) -> Decimal | None:
    if value_type not in {"integer", "number"}:
        return None
    try:
        return Decimal(str(value).strip().replace(",", ""))
    except (InvalidOperation, ValueError):
        return None


def _json_number(value: Decimal) -> int | float:
    return int(value) if value == value.to_integral_value() else float(value)


class _FieldStatistics:
    """Bounded full-column statistics for one source field."""

    def __init__(self, name: str, *, text: bool) -> None:
        self.name = name
        self.text = text
        self.null_count = 0
        self.type_counts: Counter[str] = Counter()
        self.value_counts: Counter[Any] = Counter()
        self.distinct_overflow = False
        self.minimum: Decimal | None = None
        self.maximum: Decimal | None = None
        self.all_numeric = True
        self.representatives: list[Any] = []

    def add(self, value: Any) -> None:
        value_type = _sample_type(value, text=self.text)
        if value_type == "null":
            self.null_count += 1
            return
        self.type_counts[value_type] += 1
        number = _numeric_value(value, value_type)
        if number is None:
            self.all_numeric = False
        else:
            self.minimum = number if self.minimum is None else min(self.minimum, number)
            self.maximum = number if self.maximum is None else max(self.maximum, number)
        key = _bounded_representative(value.strip() if isinstance(value, str) else value)
        if key in self.value_counts:
            self.value_counts[key] += 1
        elif len(self.value_counts) < MAX_TRACKED_DISTINCT_VALUES:
            self.value_counts[key] = 1
            if len(self.representatives) < MAX_REPRESENTATIVE_VALUES:
                self.representatives.append(key)
        else:
            self.distinct_overflow = True

    def profile(self) -> dict[str, Any]:
        non_null_count = sum(self.type_counts.values())
        result: dict[str, Any] = {
            "name": self.name,
            "value_types": dict(sorted(self.type_counts.items())),
            "null_count": self.null_count,
            "distinct_count": len(self.value_counts),
            "distinct_count_exact": not self.distinct_overflow,
        }
        if non_null_count and self.all_numeric and self.minimum is not None:
            result["min"] = _json_number(self.minimum)
            result["max"] = _json_number(self.maximum)
        # Counts are emitted only for repeating categories; per-value counts of
        # identifiers would reproduce the rows themselves.
        if (
            not self.distinct_overflow
            and len(self.value_counts) <= MAX_VALUE_COUNTS
            and len(self.value_counts) * 2 <= non_null_count
        ):
            result["value_counts"] = [
                {"value": value, "count": count}
                for value, count in sorted(
                    self.value_counts.items(), key=lambda item: (-item[1], str(item[0]))
                )
            ]
        else:
            result["representative_values"] = self.representatives
        return result


class _UnitStatistics:
    """Record count and per-field statistics for one tabular source unit."""

    def __init__(self, columns: list[Any], *, text: bool = False) -> None:
        self.record_count = 0
        self.formula_cell_count = 0
        self.fields = [
            (index, _FieldStatistics(str(column or "").strip(), text=text))
            for index, column in enumerate(columns)
            if str(column or "").strip()
        ]

    def add_row(self, row: list[Any]) -> None:
        self.record_count += 1
        self.formula_cell_count += sum(
            1 for value in row if isinstance(value, dict) and value.get("formula") is True
        )
        for index, field in self.fields:
            field.add(row[index] if index < len(row) else None)

    def field_profiles(self) -> list[dict[str, Any]]:
        return [field.profile() for _, field in self.fields]


def inspect_json(path: Path) -> dict[str, Any]:
    arrays: dict[str, dict[str, Any]] = {}
    object_keys: dict[str, set[str]] = {}
    administrative_metadata: dict[str, Any] = {}
    metadata_keys = {"country_code", "admin_level", "schema_version"}
    node_count = 0
    root_kind = "unknown"
    with path.open("rb") as stream:
        for prefix, event, value in _iter_json_events(stream):
            node_count += 1
            if node_count > MAX_JSON_NODES:
                raise ValueError("json_node_limit_exceeded")
            depth = _json_depth(prefix)
            if depth > MAX_JSON_DEPTH:
                raise ValueError("json_depth_limit_exceeded")
            if event == "start_map" and not prefix:
                root_kind = "object"
            elif event == "start_array" and not prefix:
                root_kind = "array"
            if event == "string" and isinstance(value, str) and len(value) > MAX_JSON_STRING:
                raise ValueError("json_string_limit_exceeded")
            if prefix in metadata_keys and event in {"string", "number", "boolean", "null"}:
                administrative_metadata[prefix] = _bounded_representative(value)
            if event == "start_array":
                array_key = prefix or "$"
                array_path = _json_array_path(array_key)
                arrays[array_key] = {
                    "path": array_path,
                    "unit_ref": array_path,
                    "array_prefix": "" if array_key == "$" else array_key,
                    "record_count": 0,
                    "record_count_exact": True,
                    "non_object_item_count": 0,
                    "fields": [],
                }
            if event == "map_key":
                object_keys.setdefault(prefix, set()).add(str(value)[:256])
                if len(object_keys[prefix]) > MAX_XLSX_COLUMNS:
                    raise ValueError("source_column_limit_exceeded")
            if len(arrays) > 256 or len(object_keys) > 256:
                raise ValueError("json_structure_limit_exceeded")

    # ijson's event stream is used above for limits and structure.  A second
    # streaming pass reduces every item of each discovered array to bounded
    # per-field statistics.  The full JSON document is never loaded into memory
    # or sent to the model.
    for array_prefix, array in arrays.items():
        item_prefix = "item" if array_prefix == "$" else f"{array_prefix}.item"
        fields: dict[str, _FieldStatistics] = {}
        with path.open("rb") as stream:
            try:
                for index, item in enumerate(ijson.items(stream, item_prefix)):
                    if index >= MAX_NORMALIZE_ROWS:
                        raise ValueError("source_row_limit_exceeded")
                    array["record_count"] = index + 1
                    if not isinstance(item, dict):
                        array["non_object_item_count"] += 1
                        continue
                    for key, value in item.items():
                        name = str(key)[:256]
                        if name not in fields:
                            if len(fields) >= MAX_XLSX_COLUMNS:
                                raise ValueError("source_column_limit_exceeded")
                            fields[name] = _FieldStatistics(name, text=False)
                        fields[name].add(value)
            except (ijson.common.IncompleteJSONError, ijson.common.JSONError) as error:
                raise ValueError("invalid_json") from error
        object_count = array["record_count"] - array["non_object_item_count"]
        for field in fields.values():
            # A key absent from an object item counts as a null for that field.
            field.null_count = object_count - sum(field.type_counts.values())
        array["fields"] = [fields[name].profile() for name in sorted(fields)]
    return {
        "kind": "json",
        "tree": {"kind": root_kind},
        "object_keys": {path: sorted(keys) for path, keys in object_keys.items()},
        "arrays": list(arrays.values()),
        "node_count": node_count,
        "administrative_metadata": administrative_metadata,
    }


def _json_array_path(prefix: str) -> str:
    """Convert ijson's prefix into a stable path retaining nested item axes."""

    if prefix in {"", "$"}:
        return "$"
    path = "$"
    for token in prefix.split("."):
        if token == "item":
            path += "[*]"
        elif token:
            path += f".{token}"
    return path


def inspect_xlsx(path: Path) -> dict[str, Any]:
    with zipfile.ZipFile(path) as archive:
        infos = archive.infolist()
        if len(infos) > 2048:
            raise ValueError("xlsx_entry_limit_exceeded")
        expanded = sum(info.file_size for info in infos)
        if expanded > 512 * 1024 * 1024:
            raise ValueError("xlsx_expanded_size_limit_exceeded")
        if any(
            info.file_size and info.compress_size and info.file_size / info.compress_size > 100
            for info in infos
        ):
            raise ValueError("xlsx_compression_ratio_limit_exceeded")
        names = {info.filename for info in infos}
        if "xl/vbaProject.bin" in names or any(
            name.startswith("xl/externalLinks/") or name.endswith("externalLink1.xml")
            for name in names
        ):
            raise ValueError("unsupported_source_security_feature")
        _xlsx_shared_strings(archive)
    try:
        workbook = load_workbook(
            path,
            read_only=True,
            data_only=False,
            keep_links=False,
        )
    except Exception as error:
        # Do not echo the Runtime Workspace path into an Agent message.
        raise ValueError(f"xlsx_parse_failed:{type(error).__name__}") from error
    summaries = []
    try:
        if len(workbook.worksheets) > MAX_XLSX_SHEETS:
            raise ValueError("xlsx_sheet_limit_exceeded")
        for worksheet in workbook.worksheets:
            header: list[Any] | None = None
            statistics: _UnitStatistics | None = None
            for row in worksheet.iter_rows(values_only=False):
                values = []
                if len(row) > MAX_XLSX_COLUMNS:
                    raise ValueError("source_column_limit_exceeded")
                for cell in row:
                    value = cell.value
                    if isinstance(value, str) and value.startswith("="):
                        value = {"formula": True, "display": value[:MAX_JSON_STRING]}
                    values.append(value)
                if any(value not in (None, "") for value in values):
                    if header is None:
                        header = values
                        statistics = _UnitStatistics(header)
                    else:
                        if statistics.record_count >= MAX_NORMALIZE_ROWS:
                            raise ValueError("source_row_limit_exceeded")
                        statistics.add_row(values)
            if header is not None and statistics is not None:
                summaries.append(
                    {
                        "sheet": worksheet.title,
                        "unit_ref": f"sheet:{worksheet.title}",
                        "columns": [str(value or "").strip() for value in header],
                        "fields": statistics.field_profiles(),
                        "record_count": statistics.record_count,
                        "record_count_exact": True,
                        "formula_cell_count": statistics.formula_cell_count
                        + sum(
                            1
                            for value in header
                            if isinstance(value, dict) and value.get("formula") is True
                        ),
                    }
                )
    finally:
        workbook.close()
    return {"kind": "workbook", "sheets": summaries}


def _xlsx_shared_strings(archive: zipfile.ZipFile) -> list[str]:
    try:
        root = ElementTree.fromstring(archive.read("xl/sharedStrings.xml"))
    except KeyError:
        return []
    return ["".join(node.itertext())[:MAX_JSON_STRING] for node in root]


def read_unit_rows(
    root: Path,
    relative_path: str,
    unit_ref: str,
    *,
    locator: Mapping[str, Any],
) -> list[dict[str, Any]]:
    """Read every record from one exact inspection-owned source unit."""

    path = _validated_source_path(root, relative_path)
    suffix = path.suffix.lower()
    if suffix == ".csv":
        if unit_ref != "table":
            raise ValueError("source_unit_not_found")
        return _read_csv_rows(path)
    if suffix == ".xlsx":
        sheet_name = locator.get("sheet")
        if not isinstance(sheet_name, str) or not sheet_name:
            raise ValueError("source_unit_not_found")
        return _read_xlsx_sheet_rows(path, sheet_name)
    if suffix == ".json":
        array_prefix = locator.get("array_prefix")
        if not isinstance(array_prefix, str):
            raise ValueError("source_unit_not_found")
        return _read_json_array_rows(path, array_prefix)
    raise ValueError("source_unit_not_found")


def _read_csv_rows(path: Path) -> list[dict[str, Any]]:
    sample = decode_text(_read_prefix(path, 256 * 1024)).splitlines()[:20]
    try:
        delimiter = csv.Sniffer().sniff("\n".join(sample[:10]), delimiters=",;\t|").delimiter
    except csv.Error:
        delimiter = ","
    with path.open("rb") as binary:
        text_stream = io.TextIOWrapper(
            binary, encoding=detect_encoding(_read_prefix(path, 64 * 1024)), errors="strict"
        )
        rows = csv.DictReader(text_stream, delimiter=delimiter)
        headers = [str(header).strip() for header in rows.fieldnames or []]
        if len(headers) != len(set(headers)):
            raise ValueError("source_unit_duplicate_columns")
        bounded = [
            {str(key).strip(): value for key, value in row.items() if key is not None}
            for row in itertools.islice(rows, MAX_NORMALIZE_ROWS + 1)
        ]
        if len(bounded) > MAX_NORMALIZE_ROWS:
            raise ValueError("source_row_limit_exceeded")
        return bounded


def _read_xlsx_sheet_rows(path: Path, sheet_name: str) -> list[dict[str, Any]]:
    workbook = load_workbook(path, read_only=True, data_only=False, keep_links=False)
    try:
        worksheet = next(
            (item for item in workbook.worksheets if item.title == sheet_name), None
        )
        if worksheet is None:
            raise ValueError("source_unit_not_found")
        rows = worksheet.iter_rows(values_only=False)
        header_cells = next(rows, None)
        header = [cell.value for cell in header_cells] if header_cells else None
        if not header:
            return []
        if len(header) > MAX_XLSX_COLUMNS:
            raise ValueError("source_column_limit_exceeded")
        columns = [str(value or "").strip() for value in header]
        if len([column for column in columns if column]) != len(set(column for column in columns if column)):
            raise ValueError("source_unit_duplicate_columns")
        records: list[dict[str, Any]] = []
        for cells in rows:
            values = [
                {
                    "__formula__": True,
                    "display": str(cell.value)[:MAX_JSON_STRING],
                }
                if cell.data_type == "f" or (isinstance(cell.value, str) and cell.value.startswith("="))
                else cell.value
                for cell in cells
            ]
            if not any(value not in (None, "") for value in values):
                continue
            records.append(
                {
                    column: values[index] if index < len(values) else ""
                    for index, column in enumerate(columns)
                    if column
                }
            )
            if len(records) > MAX_NORMALIZE_ROWS:
                raise ValueError("source_row_limit_exceeded")
        return records
    finally:
        workbook.close()


def _read_json_array_rows(path: Path, array_prefix: str) -> list[dict[str, Any]]:
    item_prefix = "item" if not array_prefix else f"{array_prefix}.item"
    records: list[dict[str, Any]] = []
    with path.open("rb") as stream:
        try:
            for value in ijson.items(stream, item_prefix):
                if not isinstance(value, dict):
                    raise ValueError("source_unit_item_not_object")
                records.append(value)
                if len(records) > MAX_NORMALIZE_ROWS:
                    raise ValueError("source_row_limit_exceeded")
        except ijson.common.IncompleteJSONError as error:
            raise ValueError("invalid_json") from error
    return records


def read_json_document(root: Path, relative_path: str) -> dict[str, Any]:
    """Read one bounded JSON document from an authorized Workspace source."""
    path = _validated_source_path(root, relative_path)
    if path.suffix.lower() != ".json":
        raise ValueError("workspace_source_must_be_json")
    if path.stat().st_size > 8 * 1024 * 1024:
        raise ValueError("json_source_exceeds_size_limit")
    with path.open("r", encoding=detect_encoding(_read_prefix(path, 64 * 1024))) as stream:
        payload = json.load(stream)
    if not isinstance(payload, dict):
        raise ValueError("json_source_must_be_object")
    return payload


def read_json_document_with_sha256(root: Path, relative_path: str) -> tuple[dict[str, Any], str]:
    """Read one validated JSON source and return its exact byte identity."""
    path = _validated_source_path(root, relative_path)
    if path.suffix.lower() != ".json":
        raise ValueError("workspace_source_must_be_json")
    if path.stat().st_size > 8 * 1024 * 1024:
        raise ValueError("json_source_exceeds_size_limit")
    raw = path.read_bytes()
    try:
        payload = json.loads(raw.decode(detect_encoding(raw[: 64 * 1024])))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("workspace_json_invalid") from error
    if not isinstance(payload, dict):
        raise ValueError("json_source_must_be_object")
    return payload, hashlib.sha256(raw).hexdigest()


def _read_prefix(path: Path, limit: int) -> bytes:
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        return raw[:limit]
    return raw


def detect_encoding(raw: bytes) -> str:
    for encoding in ("utf-8-sig", "utf-16", "utf-16-le", "utf-16-be"):
        try:
            raw.decode(encoding)
            return encoding
        except UnicodeDecodeError:
            continue
    best = from_bytes(raw).best()
    if best is not None and best.encoding and best.percent_chaos < 10:
        return best.encoding
    raise ValueError("unsupported_text_encoding")


def decode_text(raw: bytes) -> str:
    return raw.decode(detect_encoding(raw))


def _iter_json_events(stream: BinaryIO):
    try:
        yield from ijson.parse(stream)
    except (ijson.common.IncompleteJSONError, ijson.common.JSONError) as error:
        raise ValueError("invalid_json") from error


def _json_depth(prefix: str) -> int:
    return sum(1 for token in prefix.split(".") if token and token != "item")
