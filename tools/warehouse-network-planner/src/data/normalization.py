"""Deterministic normalization of selected Workspace mappings into Case rows."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Literal

from pydantic import ValidationError
from supply_chain_planner.data.mapping import SourceRole, TransformRegistry, TransformSpec
from supply_chain_planner.network.models import (
    CurrentAssignmentRecord,
    DataQualityIssue,
    DemandCityRecord,
    NormalizedInputBatch,
    ProvidedRouteFactRecord,
    RouteQuoteRecord,
    WarehouseRecord,
)

NormalizationState = Literal["ready", "needs_input", "needs_geography"]


@dataclass(frozen=True)
class ConfirmedFieldMapping:
    target_field: str
    source_field: str
    transform: TransformSpec


@dataclass(frozen=True)
class ConfirmedSourceRows:
    """Current validated rows plus caller-confirmed domain field mappings."""

    role: SourceRole | str
    rows: Sequence[Mapping[str, object]]
    mappings: Sequence[ConfirmedFieldMapping]
    relative_path: str = ""
    unit_ref: str = ""
    selection_key: str = ""


class RowNormalizationError(ValueError):
    def __init__(self, code: str, field_name: str | None = None):
        super().__init__(code)
        self.code = code
        self.field_name = field_name


class NetworkInputValidator:
    def validate(self, batch: NormalizedInputBatch) -> list[DataQualityIssue]:
        issues = list(batch.issues)
        repeated_errors: dict[tuple[str, str], int] = {}
        demand_ids = [row.city_id for row in batch.demand_cities]
        warehouse_ids = [row.warehouse_id for row in batch.warehouses]
        if not demand_ids:
            issues.append(self._error("demand_missing", "没有找到可用于规划的需求城市数据。"))
        if not warehouse_ids:
            issues.append(self._error("warehouse_missing", "没有找到已有仓库数据。"))
        if len(set(demand_ids)) != len(demand_ids):
            issues.append(self._error("demand_city_duplicate", "需求城市标识必须唯一。"))
        if len(set(warehouse_ids)) != len(warehouse_ids):
            issues.append(self._error("warehouse_duplicate", "仓库标识必须唯一。"))
        demand_set = set(demand_ids)
        existing_warehouse_set = {
            row.warehouse_id for row in batch.warehouses if row.is_existing
        }
        assignment_ids = [row.demand_city_id for row in batch.current_assignments]
        if len(set(assignment_ids)) != len(assignment_ids):
            issues.append(
                self._error("current_assignment_duplicate", "每个需求城市只能有一个当前服务仓。")
            )
        for row in batch.current_assignments:
            if row.demand_city_id not in demand_set:
                repeated_errors[("assignment_demand_unknown", "demand_city_id")] = (
                    repeated_errors.get(("assignment_demand_unknown", "demand_city_id"), 0) + 1
                )
            if row.serving_warehouse_id not in existing_warehouse_set:
                repeated_errors[("assignment_warehouse_unknown", "serving_warehouse_id")] = (
                    repeated_errors.get(("assignment_warehouse_unknown", "serving_warehouse_id"), 0) + 1
                )
        center_ids = {
            row.warehouse_id
            for row in batch.warehouses
            if row.warehouse_type == "center"
        }
        for row in batch.warehouses:
            if row.warehouse_type == "cross_docking" and row.upstream_center_id is None:
                repeated_errors[("warehouse_upstream_center_required", "upstream_center_id")] = (
                    repeated_errors.get(("warehouse_upstream_center_required", "upstream_center_id"), 0) + 1
                )
            if (
                row.warehouse_type == "cross_docking"
                and row.upstream_center_id is not None
                and row.upstream_center_id not in center_ids
            ):
                repeated_errors[("warehouse_upstream_center_unknown", "upstream_center_id")] = (
                    repeated_errors.get(("warehouse_upstream_center_unknown", "upstream_center_id"), 0) + 1
                )
        quote_keys = [
            (row.origin_id, row.destination_id, row.layer) for row in batch.route_quotes
        ]
        if len(set(quote_keys)) != len(quote_keys):
            issues.append(self._error("route_quote_duplicate", "路线报价键必须唯一。"))
        provided_fact_keys = [
            (row.origin_id, row.destination_id, row.layer)
            for row in batch.provided_route_facts
        ]
        if len(set(provided_fact_keys)) != len(provided_fact_keys):
            issues.append(
                self._error("provided_route_fact_duplicate", "已提供路线事实键必须唯一。")
            )
        if not batch.current_assignments:
            issues.append(
                DataQualityIssue(
                    code="current_assignment_missing",
                    severity="warning",
                    business_message="没有发现当前覆盖方案；当前分析将使用优化基线。",
                )
            )
        issues.extend(self._repeated_errors(repeated_errors))
        return issues

    @staticmethod
    def _error(code: str, message: str) -> DataQualityIssue:
        return DataQualityIssue(code=code, severity="error", business_message=message)

    @staticmethod
    def _repeated_errors(counts: dict[tuple[str, str], int]) -> list[DataQualityIssue]:
        messages = {
            "assignment_demand_unknown": "当前覆盖中有需求城市不在需求列表中。",
            "assignment_warehouse_unknown": "当前覆盖中有服务仓不在已有仓列表中。",
            "warehouse_upstream_center_unknown": "当前覆盖中有前置仓引用了不存在的中心仓。",
            "warehouse_upstream_center_required": "有越库仓没有填写上游中心仓，无法计算干线路线；请补充上游中心仓。",
        }
        return [
            DataQualityIssue(
                code=code,
                severity="error",
                business_message=f"{messages[code]} 问题记录数：{count}。",
                field_name=field_name,
            )
            for (code, field_name), count in counts.items()
        ]


def normalize_confirmed_rows(
    sources: Sequence[ConfirmedSourceRows],
) -> tuple[NormalizationState, NormalizedInputBatch]:
    """Normalize verified rows without reading files or persisting Case state."""

    transforms = TransformRegistry()
    demands: list[DemandCityRecord] = []
    warehouses: list[WarehouseRecord] = []
    assignments: list[CurrentAssignmentRecord] = []
    quotes: list[RouteQuoteRecord] = []
    provided_route_facts: list[ProvidedRouteFactRecord] = []
    issues: list[DataQualityIssue] = []
    duplicate_counts: dict[tuple[str, str, str, str], int] = {}
    row_error_counts: dict[tuple[str, str, str, str], int] = {}
    for source in sources:
        try:
            role = SourceRole(source.role)
        except ValueError:
            issues.append(
                DataQualityIssue(
                    code="normalization_role_unknown",
                    severity="error",
                    business_message="输入包含无法识别的数据角色。",
                    source_id=_source_id(source),
                    field_name="role",
                )
            )
            continue
        if role == SourceRole.ADMINISTRATIVE_CATALOG:
            issues.append(
                DataQualityIssue(
                    code="normalization_role_unsupported",
                    severity="error",
                    business_message="行政区目录应由地理准备工具处理，不能作为仓网记录标准化。",
                    source_id=_source_id(source),
                    field_name=role.value,
                )
            )
            continue
        local_demands: list[DemandCityRecord] = []
        local_warehouses: list[WarehouseRecord] = []
        local_assignments: list[CurrentAssignmentRecord] = []
        local_quotes: list[RouteQuoteRecord] = []
        local_facts: list[ProvidedRouteFactRecord] = []
        for row_index, row in enumerate(source.rows, start=1):
            try:
                values = _map_confirmed_row(row, source.mappings, transforms)
                _append_normalized_record(
                    role,
                    values,
                    local_demands,
                    local_warehouses,
                    local_assignments,
                    local_quotes,
                    local_facts,
                )
            except RowNormalizationError as error:
                error_key = (
                    source.selection_key,
                    error.code,
                    error.field_name or "record",
                    role.value,
                )
                row_error_counts[error_key] = row_error_counts.get(error_key, 0) + 1
            except (KeyError, ValueError, ValidationError) as error:
                error_key = (
                    source.selection_key,
                    f"{role.value}_row_invalid",
                    _validation_field(error),
                    role.value,
                )
                row_error_counts[error_key] = row_error_counts.get(error_key, 0) + 1
        _merge_records(
            demands,
            local_demands,
            lambda record: record.city_id,
            "demand_city",
            source,
            duplicate_counts,
        )
        _merge_records(
            warehouses,
            local_warehouses,
            lambda record: record.warehouse_id,
            "warehouse",
            source,
            duplicate_counts,
        )
        _merge_records(
            assignments,
            local_assignments,
            lambda record: record.demand_city_id,
            "current_assignment",
            source,
            duplicate_counts,
        )
        _merge_records(
            quotes,
            local_quotes,
            lambda record: (record.origin_id, record.destination_id, record.layer),
            "route_quote",
            source,
            duplicate_counts,
        )
        _merge_records(
            provided_route_facts,
            local_facts,
            lambda record: (record.origin_id, record.destination_id, record.layer),
            "provided_route_fact",
            source,
            duplicate_counts,
        )
    issues.extend(_row_error_issues(row_error_counts, sources))
    issues.extend(_duplicate_issues(duplicate_counts))
    batch = NormalizedInputBatch(
        demand_cities=demands,
        warehouses=warehouses,
        current_assignments=assignments,
        route_quotes=quotes,
        provided_route_facts=provided_route_facts,
        issues=issues,
    )
    validated = batch.model_copy(update={"issues": NetworkInputValidator().validate(batch)})
    if any(issue.severity == "error" for issue in validated.issues):
        return "needs_input", validated
    if any(
        record.longitude is None or record.latitude is None
        for record in [*validated.demand_cities, *validated.warehouses]
    ):
        return "needs_geography", validated
    return "ready", validated


def _source_id(source: ConfirmedSourceRows) -> str | None:
    return source.selection_key[:64] or None


def _validation_field(error: Exception) -> str:
    if isinstance(error, ValidationError):
        first = error.errors()[0] if error.errors() else {}
        location = first.get("loc", ())
        return str(location[0])[:256] if location else "record"
    return "record"


def _row_error_issues(
    counts: dict[tuple[str, str, str, str], int],
    sources: Sequence[ConfirmedSourceRows],
) -> list[DataQualityIssue]:
    source_ids = {source.selection_key: source for source in sources}
    return [
        DataQualityIssue(
            code=code,
            severity="error",
            business_message=f"{role} 数据存在缺少必需字段或格式不正确的记录。问题记录数：{count}。",
            source_id=source_key[:64] or None,
            field_name=field_name,
        )
        for (source_key, code, field_name, role), count in counts.items()
        if not source_key or source_key in source_ids
    ]


def _merge_records(
    target: list[object],
    incoming: Sequence[object],
    key_fn,
    label: str,
    source: ConfirmedSourceRows,
    duplicate_counts: dict[tuple[str, str, str, str], int],
) -> None:
    existing = {key_fn(record): record for record in target}
    for record in incoming:
        key = key_fn(record)
        prior = existing.get(key)
        if prior is None:
            target.append(record)
            existing[key] = record
            continue
        if prior == record:
            duplicate_key = (
                f"{label}_duplicate_identical",
                source.selection_key or source.relative_path,
                source.unit_ref,
                "warning",
            )
            duplicate_counts[duplicate_key] = duplicate_counts.get(duplicate_key, 0) + 1
        else:
            duplicate_key = (
                f"{label}_duplicate_conflict",
                source.selection_key or source.relative_path,
                source.unit_ref,
                "error",
            )
            duplicate_counts[duplicate_key] = duplicate_counts.get(duplicate_key, 0) + 1


def _duplicate_issues(
    duplicate_counts: dict[tuple[str, str, str, str], int]
) -> list[DataQualityIssue]:
    issues: list[DataQualityIssue] = []
    for (code, source_key, unit_ref, severity), count in duplicate_counts.items():
        message = (
            "选中的来源包含完全相同的重复记录，已稳定保留首次记录。"
            if severity == "warning"
            else "选中的来源包含相同业务标识但内容冲突的记录。"
        )
        issues.append(
            DataQualityIssue(
                code=code,
                severity=severity,
                business_message=f"{message} 重复记录数：{count}。",
                source_id=source_key[:64] or None,
                field_name=unit_ref[:256] or None,
            )
        )
    return issues


def _map_confirmed_row(
    row: Mapping[str, object],
    mappings: Sequence[ConfirmedFieldMapping],
    transforms: TransformRegistry,
) -> dict[str, object]:
    values: dict[str, object] = {}
    for mapping in mappings:
        value = row.get(mapping.source_field)
        if isinstance(value, Mapping) and value.get("__formula__") is True:
            raise RowNormalizationError("formula_value_not_materialized", mapping.target_field)
        if value in (None, "") and "[]." in mapping.source_field:
            value = row.get(mapping.source_field.rsplit("[].", 1)[-1])
        if value in (None, ""):
            continue
        values[mapping.target_field] = transforms.apply(mapping.transform, value)
    return values


def _append_normalized_record(
    role: SourceRole,
    values: Mapping[str, object],
    demands: list[DemandCityRecord],
    warehouses: list[WarehouseRecord],
    assignments: list[CurrentAssignmentRecord],
    quotes: list[RouteQuoteRecord],
    provided_route_facts: list[ProvidedRouteFactRecord],
) -> None:
    if role == SourceRole.DEMAND:
        demands.append(
            DemandCityRecord(
                city_id=values["city_id"],
                city_name=values["city_name"],
                province_id=values.get("province_id"),
                province_name=values.get("province_name"),
                demand_quantity=values["demand_quantity"],
                longitude=values.get("longitude"),
                latitude=values.get("latitude"),
            )
        )
        return
    if role in {SourceRole.EXISTING_WAREHOUSE, SourceRole.CANDIDATE_WAREHOUSE}:
        is_existing = role == SourceRole.EXISTING_WAREHOUSE
        warehouses.append(
            WarehouseRecord(
                warehouse_id=values["warehouse_id"],
                warehouse_name=values["warehouse_name"],
                warehouse_type=values["warehouse_type"],
                city_id=values["city_id"],
                city_name=values["city_name"],
                province_id=values.get("province_id"),
                province_name=values.get("province_name"),
                longitude=values.get("longitude"),
                latitude=values.get("latitude"),
                upstream_center_id=values.get("upstream_center_id"),
                is_existing=is_existing,
                is_fixed=values.get("is_fixed"),
            )
        )
        return
    if role == SourceRole.CURRENT_ASSIGNMENT:
        assignments.append(
            CurrentAssignmentRecord(
                demand_city_id=values["demand_city_id"],
                serving_warehouse_id=values["serving_warehouse_id"],
                upstream_center_id=values.get("upstream_center_id"),
            )
        )
        return
    if role == SourceRole.ROUTE_QUOTE:
        for field_name in ("layer", "currency", "vehicle_capacity"):
            if values.get(field_name) in (None, ""):
                raise RowNormalizationError(
                    f"route_quote_{field_name}_missing",
                    field_name,
                )
        route_fact_fields = ("distance_km", "duration_hours", "method")
        supplied_route_fact_fields = {
            field_name
            for field_name in route_fact_fields
            if values.get(field_name) not in (None, "")
        }
        if supplied_route_fact_fields and supplied_route_fact_fields != set(
            route_fact_fields
        ):
            missing = sorted(set(route_fact_fields) - supplied_route_fact_fields)
            raise RowNormalizationError(
                "provided_route_fact_incomplete",
                ",".join(missing),
            )
        quotes.append(
            RouteQuoteRecord(
                origin_id=values["origin_id"],
                destination_id=values["destination_id"],
                layer=str(values["layer"]).lower(),
                price_per_vehicle=values["price_per_vehicle"],
                currency=str(values["currency"]).upper(),
                vehicle_capacity=values["vehicle_capacity"],
            )
        )
        if supplied_route_fact_fields:
            provided_route_facts.append(
                ProvidedRouteFactRecord(
                    origin_id=values["origin_id"],
                    destination_id=values["destination_id"],
                    destination_name=values.get("destination_name"),
                    layer=str(values["layer"]).lower(),
                    distance_km=values["distance_km"],
                    duration_hours=values["duration_hours"],
                    source_method=str(values["method"]).strip(),
                )
            )
        return
    raise RowNormalizationError("normalization_role_unsupported", role.value)
