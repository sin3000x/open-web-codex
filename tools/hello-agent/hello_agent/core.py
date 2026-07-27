"""Structured greeting contract and deterministic business rules."""

from __future__ import annotations

from pydantic import BaseModel

MAX_NAME_CHARACTERS = 40


class Greeting(BaseModel):
    """One structured greeting returned by the Tool."""

    name: str
    message: str


def build_greeting(name: str) -> Greeting:
    """Validate one name and return its canonical greeting."""

    clean_name = name.strip()
    if not clean_name:
        raise ValueError("name must not be empty")
    if len(clean_name) > MAX_NAME_CHARACTERS:
        raise ValueError(
            f"name must contain at most {MAX_NAME_CHARACTERS} characters"
        )
    return Greeting(name=clean_name, message=f"你好，{clean_name}！")
