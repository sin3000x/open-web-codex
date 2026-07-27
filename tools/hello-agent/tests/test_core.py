from __future__ import annotations

import pytest

from hello_agent.core import Greeting, build_greeting


def test_normalizes_name_and_builds_canonical_message() -> None:
    greeting = build_greeting(" 小林 ")

    assert greeting == Greeting(name="小林", message="你好，小林！")


@pytest.mark.parametrize("name", ["", "   ", "x" * 41])
def test_rejects_invalid_names(name: str) -> None:
    with pytest.raises(ValueError):
        build_greeting(name)
