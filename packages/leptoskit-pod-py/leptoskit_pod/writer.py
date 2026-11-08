from __future__ import annotations

import struct
from typing import Iterable

from leptoskit_pod.errors import TooLarge


class Writer:
    """Appends little-endian Pod fields."""

    __slots__ = ("_bytes",)

    def __init__(self) -> None:
        self._bytes = bytearray()

    @classmethod
    def new(cls) -> Writer:
        return cls()

    @classmethod
    def with_capacity(cls, capacity: int) -> Writer:
        w = cls()
        w._bytes = bytearray(capacity)
        w._bytes.clear()
        return w

    def len(self) -> int:
        return len(self._bytes)

    def is_empty(self) -> bool:
        return not self._bytes

    def raw(self, value: bytes | Iterable[int]) -> None:
        self._bytes.extend(value)

    def u8(self, value: int) -> None:
        self._bytes.append(value & 0xFF)

    def u16(self, value: int) -> None:
        self._bytes.extend(struct.pack("<H", value & 0xFFFF))

    def u32(self, value: int) -> None:
        self._bytes.extend(struct.pack("<I", value & 0xFFFFFFFF))

    def u64(self, value: int) -> None:
        self._bytes.extend(struct.pack("<Q", value & 0xFFFFFFFFFFFFFFFF))

    def i8(self, value: int) -> None:
        self.u8(value)

    def i16(self, value: int) -> None:
        self.u16(value)

    def i32(self, value: int) -> None:
        self.u32(value)

    def i64(self, value: int) -> None:
        self.u64(value)

    def bool(self, value: bool) -> None:
        self.u8(1 if value else 0)

    def bytes(self, value: bytes) -> None:
        if len(value) > 0xFFFFFFFF:
            raise TooLarge()
        self.u32(len(value))
        self.raw(value)

    def string(self, value: str) -> None:
        self.bytes(value.encode("utf-8"))

    def vec_len(self, length: int) -> None:
        if length > 0xFFFFFFFF:
            raise TooLarge()
        self.u32(length)

    def option_tag(self, some: bool) -> None:
        self.u8(1 if some else 0)

    def result_tag(self, ok: bool) -> None:
        self.u8(0 if ok else 1)

    def struct_header(self, disc: int, version: int) -> None:
        self.u16(disc)
        self.u16(version)

    def variant(self, disc: int) -> None:
        self.u16(disc)

    def as_slice(self) -> bytes:
        return bytes(self._bytes)

    def finish(self) -> bytes:
        return self.as_slice()
