from __future__ import annotations

from collections.abc import Callable
from typing import TypeVar

from leptoskit_pod.errors import UnknownVersion
from leptoskit_pod.reader import Reader
from leptoskit_pod.writer import Writer

T = TypeVar("T")


def encode_struct(version: int, encode_fields: Callable[[Writer], None]) -> bytes:
    w = Writer.new()
    w.struct_header(version)
    encode_fields(w)
    return w.finish()


def encode_struct_v0(encode_fields: Callable[[Writer], None]) -> bytes:
    return encode_struct(0, encode_fields)


def decode_struct_version(
    r: Reader,
    expected_version: int,
    decode_fields: Callable[[Reader], T],
) -> T:
    version = r.struct_header()
    if version != expected_version:
        raise UnknownVersion(version)
    return decode_fields(r)
