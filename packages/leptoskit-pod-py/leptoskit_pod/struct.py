from __future__ import annotations

from collections.abc import Callable
from typing import TypeVar

from leptoskit_pod.errors import InvalidTag, UnknownVersion
from leptoskit_pod.reader import Reader
from leptoskit_pod.writer import Writer

T = TypeVar("T")


def encode_struct_v0(encode_fields: Callable[[Writer], None]) -> bytes:
    w = Writer.new()
    w.struct_header(0, 0)
    encode_fields(w)
    return w.finish()


def decode_struct_version(
    r: Reader,
    expected_version: int,
    decode_fields: Callable[[Reader], T],
) -> T:
    version, flags = r.struct_header()
    if version != expected_version:
        raise UnknownVersion(version)
    if flags != 0:
        raise InvalidTag(flags)
    return decode_fields(r)
