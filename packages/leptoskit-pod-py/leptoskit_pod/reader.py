from __future__ import annotations

import struct

from leptoskit_pod.errors import InvalidTag, InvalidUtf8, Trailing, Truncated


class Reader:
    """Reads little-endian Pod fields from a buffer."""

    __slots__ = ("_remaining",)

    def __init__(self, input: bytes) -> None:
        self._remaining = memoryview(input)

    def remaining(self) -> memoryview:
        return self._remaining

    def _take(self, length: int) -> memoryview:
        if len(self._remaining) < length:
            raise Truncated()
        out = self._remaining[:length]
        self._remaining = self._remaining[length:]
        return out

    def u8(self) -> int:
        return self._take(1)[0]

    def u16(self) -> int:
        return struct.unpack("<H", self._take(2))[0]

    def u32(self) -> int:
        return struct.unpack("<I", self._take(4))[0]

    def u64(self) -> int:
        return struct.unpack("<Q", self._take(8))[0]

    def i8(self) -> int:
        v = self.u8()
        return (v ^ 0x80) - 0x80 if v >= 0x80 else v

    def i16(self) -> int:
        v = self.u16()
        return v - 0x10000 if v >= 0x8000 else v

    def i32(self) -> int:
        v = self.u32()
        return v - 0x100000000 if v >= 0x80000000 else v

    def i64(self) -> int:
        v = self.u64()
        return v - (1 << 64) if v >= (1 << 63) else v

    def bool(self) -> bool:
        tag = self.u8()
        if tag == 0:
            return False
        if tag == 1:
            return True
        raise InvalidTag(tag)

    def bytes(self) -> bytes:
        length = self.u32()
        return bytes(self._take(length))

    def string(self) -> str:
        raw = self.bytes()
        try:
            return raw.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise InvalidUtf8() from exc

    def vec_len(self) -> int:
        return self.u32()

    def option_tag(self) -> bool:
        tag = self.u8()
        if tag == 0:
            return False
        if tag == 1:
            return True
        raise InvalidTag(tag)

    def result_tag(self) -> bool:
        tag = self.u8()
        if tag == 0:
            return True
        if tag == 1:
            return False
        raise InvalidTag(tag)

    def struct_header(self) -> int:
        return self.u16()

    def variant(self) -> int:
        return self.u16()

    def finish(self) -> None:
        if len(self._remaining) != 0:
            raise Trailing()
