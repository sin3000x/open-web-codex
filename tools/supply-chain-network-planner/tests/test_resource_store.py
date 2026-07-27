from __future__ import annotations

import json

import pytest

from supply_chain_planner.resource_store import ResourceStore, data_ref


def test_resource_store_is_content_addressed(tmp_path) -> None:
    store = ResourceStore(tmp_path)

    first = store.publish("example.v1", {"value": 1})
    second = store.publish("example.v1", {"value": 1})

    assert first.resource_id == second.resource_id
    assert json.loads(store.read(first.resource_id)) == {"value": 1}
    assert store.load(data_ref(first)) == {"value": 1}


def test_resource_store_rejects_path_traversal(tmp_path) -> None:
    store = ResourceStore(tmp_path)

    with pytest.raises(ValueError, match="invalid"):
        store.read("../secret")


def test_resource_store_supports_a_separate_data_agent_uri_namespace(tmp_path) -> None:
    store = ResourceStore(
        tmp_path,
        uri_prefix="supply-chain-data://resources/",
    )

    published = store.publish("planning-dataset.v1", {"value": 1})

    assert published.uri.startswith("supply-chain-data://resources/")
    assert store.load_uri(published.uri) == {"value": 1}
