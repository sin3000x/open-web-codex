"""Exact read-only queries over one prepared planning input.

Inspection statistics describe raw source columns before mapping. Business
fact questions (counts, group totals, filtered lists) are answered here over
every normalized record of the prepared file.
"""

from __future__ import annotations

from collections.abc import Callable, Sequence
from decimal import Decimal, InvalidOperation
from types import UnionType
from typing import Any, Literal, Union, get_args, get_origin

from supply_chain_planner.network.models import (
    CurrentAssignmentRecord,
    DemandCityRecord,
    NetworkRecord,
    PlanningInputIdentity,
    ProvidedRouteFactRecord,
    RouteQuoteRecord,
    WarehouseRecord,
)
from supply_chain_planner.shared.models import (
    PreparedNetworkResource,
    PreparedQueryAnswered,
    PreparedQueryDataset,
    PreparedQueryFilter,
    PreparedQueryGroup,
    PreparedQueryKey,
    PreparedQueryMeasure,
    PreparedQueryMeasureValue,
    PreparedQueryOrder,
)

FieldKind = Literal["text", "number", "bool"]

MAX_QUERY_ROWS = 100
MAX_QUERY_GROUPS = 200
_MEAN_QUANTUM = Decimal("0.000001")
_SUMMARY_GROUPS = 10

DATASET_RECORD_MODELS: dict[str, type[NetworkRecord]] = {
    "warehouses": WarehouseRecord,
    "demand_cities": DemandCityRecord,
    "current_assignments": CurrentAssignmentRecord,
    "route_quotes": RouteQuoteRecord,
    "provided_route_facts": ProvidedRouteFactRecord,
}


def _annotation_kind(annotation: Any) -> FieldKind:
    if get_origin(annotation) in (Union, UnionType):
        members = [item for item in get_args(annotation) if item is not type(None)]
    else:
        members = [annotation]
    if all(item is bool for item in members):
        return "bool"
    if all(item in (Decimal, float, int) for item in members):
        return "number"
    return "text"


DATASET_FIELDS: dict[str, dict[str, FieldKind]] = {
    dataset_name: {
        name: _annotation_kind(info.annotation) for name, info in record_model.model_fields.items()
    }
    for dataset_name, record_model in DATASET_RECORD_MODELS.items()
}


def field_catalog_description() -> str:
    """Render the dataset field catalog for the Tool description."""
    return "; ".join(
        f"{dataset_name}: " + ", ".join(f"{name}({kind})" for name, kind in fields.items())
        for dataset_name, fields in DATASET_FIELDS.items()
    )


def query_prepared_input(
    prepared: PreparedNetworkResource,
    identity: PlanningInputIdentity,
    prepared_input_relative_path: str,
    dataset: PreparedQueryDataset,
    *,
    filters: Sequence[PreparedQueryFilter] = (),
    group_by: Sequence[str] = (),
    measures: Sequence[PreparedQueryMeasure] = (),
    select: Sequence[str] = (),
    order_by: PreparedQueryOrder | None = None,
    limit: int = 20,
) -> PreparedQueryAnswered:
    """Evaluate one bounded query over every record of one prepared dataset."""
    fields = DATASET_FIELDS[dataset]
    for name in [*group_by, *select, *(item.field_name for item in filters)]:
        _require_field(dataset, name)
    if order_by is not None:
        _require_field(dataset, order_by.field_name)
    if order_by is not None and group_by:
        raise ValueError("query_order_by_requires_ungrouped_rows")
    for measure in measures:
        if _require_field(dataset, measure.field_name) != "number":
            raise ValueError(f"query_measure_requires_number_field: {measure.field_name}")
    if not 0 <= limit <= MAX_QUERY_ROWS:
        raise ValueError(f"query_limit_out_of_range: 0..{MAX_QUERY_ROWS}")

    records: list[NetworkRecord] = list(getattr(prepared, dataset))
    predicates = [_compile_predicate(fields, item) for item in filters]
    matched = [record for record in records if all(predicate(record) for predicate in predicates)]
    overall = [_measure(matched, item) for item in measures]

    groups: list[PreparedQueryGroup] = []
    group_count = 0
    rows: list[PreparedQueryKey] = []
    if group_by:
        buckets: dict[tuple[Any, ...], list[NetworkRecord]] = {}
        for record in matched:
            buckets.setdefault(tuple(getattr(record, name) for name in group_by), []).append(record)
        ordered = sorted(
            buckets.values(),
            key=lambda members: (-len(members), _sortable_key(members[0], group_by)),
        )
        group_count = len(ordered)
        groups = [
            PreparedQueryGroup(
                key=_json_values(members[0], group_by),
                count=len(members),
                measures=[_measure(members, item) for item in measures],
            )
            for members in ordered[:MAX_QUERY_GROUPS]
        ]
    else:
        selected_fields = list(select) or list(fields)
        rows = [
            _json_values(record, selected_fields) for record in _ordered(matched, order_by)[:limit]
        ]

    return PreparedQueryAnswered(
        outcome="answered",
        schemaVersion="prepared_network_query.v1",
        summary=_summary(dataset, len(records), len(matched), groups, group_count),
        prepared_input_relative_path=prepared_input_relative_path,
        input_identity=identity,
        country_code=prepared.country_code,
        roles=prepared.roles,
        role_counts=prepared.role_counts,
        dataset=dataset,
        record_count=len(records),
        matched_count=len(matched),
        measures=overall,
        groups=groups,
        group_count=group_count,
        groups_truncated=group_count > MAX_QUERY_GROUPS,
        rows=rows,
        rows_truncated=not group_by and len(matched) > len(rows),
    )


def _require_field(dataset: str, name: str) -> FieldKind:
    fields = DATASET_FIELDS[dataset]
    if name not in fields:
        raise ValueError(
            f"query_field_invalid: {name} is not a {dataset} field; allowed: {', '.join(fields)}"
        )
    return fields[name]


def _operand(kind: FieldKind, name: str, raw: str | bool | float | None) -> Any:
    if raw is None:
        return None
    if kind == "number":
        if isinstance(raw, bool):
            raise ValueError(f"query_filter_value_invalid: {name} expects a number")
        try:
            return Decimal(str(raw))
        except InvalidOperation as error:
            raise ValueError(f"query_filter_value_invalid: {name} expects a number") from error
    if kind == "bool":
        if not isinstance(raw, bool):
            raise ValueError(f"query_filter_value_invalid: {name} expects true or false")
        return raw
    if not isinstance(raw, str):
        raise ValueError(f"query_filter_value_invalid: {name} expects text")
    return raw


def _comparable(kind: FieldKind, value: Any) -> Any:
    if value is None or kind != "number":
        return value
    return Decimal(str(value))


def _compile_predicate(
    fields: dict[str, FieldKind],
    predicate_spec: PreparedQueryFilter,
) -> Callable[[NetworkRecord], bool]:
    name = predicate_spec.field_name
    kind = fields[name]
    op = predicate_spec.op
    if op in {"gt", "gte", "lt", "lte"} and kind != "number":
        raise ValueError(f"query_filter_range_requires_number_field: {name}")
    if op == "in":
        accepted = {_operand(kind, name, item) for item in predicate_spec.values or []}
        return lambda record: _comparable(kind, getattr(record, name)) in accepted
    expected = _operand(kind, name, predicate_spec.value)
    if op == "eq":
        return lambda record: _comparable(kind, getattr(record, name)) == expected
    if op == "ne":
        return lambda record: _comparable(kind, getattr(record, name)) != expected
    compare: Callable[[Any, Any], bool] = {
        "gt": lambda left, right: left > right,
        "gte": lambda left, right: left >= right,
        "lt": lambda left, right: left < right,
        "lte": lambda left, right: left <= right,
    }[op]

    def matches(record: NetworkRecord) -> bool:
        actual = _comparable(kind, getattr(record, name))
        return actual is not None and compare(actual, expected)

    return matches


def _measure(
    records: Sequence[NetworkRecord],
    measure: PreparedQueryMeasure,
) -> PreparedQueryMeasureValue:
    present = [
        Decimal(str(value))
        for value in (getattr(record, measure.field_name) for record in records)
        if value is not None
    ]
    result: Decimal | None
    if not present:
        result = None
    elif measure.op == "sum":
        result = sum(present, Decimal(0))
    elif measure.op == "min":
        result = min(present)
    elif measure.op == "max":
        result = max(present)
    else:
        result = (sum(present, Decimal(0)) / len(present)).quantize(_MEAN_QUANTUM)
    return PreparedQueryMeasureValue(
        op=measure.op,
        field_name=measure.field_name,
        value=result,
        value_count=len(present),
    )


def _ordered(
    records: list[NetworkRecord],
    order_by: PreparedQueryOrder | None,
) -> list[NetworkRecord]:
    if order_by is None:
        return records
    name = order_by.field_name
    present = [record for record in records if getattr(record, name) is not None]
    missing = [record for record in records if getattr(record, name) is None]
    present.sort(key=lambda record: getattr(record, name), reverse=order_by.descending)
    return present + missing


def _sortable_key(record: NetworkRecord, names: Sequence[str]) -> tuple[str, ...]:
    return tuple(str(getattr(record, name)) for name in names)


def _json_values(record: NetworkRecord, names: Sequence[str]) -> PreparedQueryKey:
    payload = record.model_dump(mode="json", include=set(names))
    return {name: payload.get(name) for name in names}


def _summary(
    dataset: str,
    record_count: int,
    matched_count: int,
    groups: list[PreparedQueryGroup],
    group_count: int,
) -> str:
    text = f"{matched_count} of {record_count} {dataset} records matched."
    if groups:
        shown = "; ".join(
            ", ".join(f"{name}={value}" for name, value in group.key.items()) + f": {group.count}"
            for group in groups[:_SUMMARY_GROUPS]
        )
        more = f" (+{group_count - _SUMMARY_GROUPS} more)" if group_count > _SUMMARY_GROUPS else ""
        text += f" Groups: {shown}{more}."
    return text
