from __future__ import annotations

from enum import IntEnum

from leptoskit_pod.errors import InvalidEnvelope, InvalidTag, TooLarge, Truncated
from leptoskit_pod.reader import Reader
from leptoskit_pod.writer import Writer

MAGIC = b"LK01"
CONTENT_TYPE = "application/vnd.leptoskit.rpc.v1+bin"
CODEC_VERSION = 1


class MessageType(IntEnum):
    Rpc = 1
    Live = 2


def _message_type_from_u16(value: int) -> MessageType:
    if value == 1:
        return MessageType.Rpc
    if value == 2:
        return MessageType.Live
    raise InvalidTag(value)


class Envelope:
    __slots__ = ("codec_ver", "msg_type", "flags", "body")

    def __init__(self, codec_ver: int, msg_type: MessageType, flags: int, body: bytes) -> None:
        self.codec_ver = codec_ver
        self.msg_type = msg_type
        self.flags = flags
        self.body = body


def wrap(msg_type: MessageType, flags: int, body: bytes) -> bytes:
    if len(body) > 0xFFFFFFFF:
        raise TooLarge()
    w = Writer.with_capacity(14 + len(body))
    w.raw(MAGIC)
    w.u16(CODEC_VERSION)
    w.u16(int(msg_type))
    w.u16(flags)
    w.u32(len(body))
    w.raw(body)
    return w.finish()


def unwrap(data: bytes) -> Envelope:
    if len(data) < 14:
        raise Truncated()
    if data[:4] != MAGIC:
        raise InvalidEnvelope()
    r = Reader(data[4:])
    codec_ver = r.u16()
    if codec_ver != CODEC_VERSION:
        raise InvalidEnvelope()
    msg_type = _message_type_from_u16(r.u16())
    flags = r.u16()
    body_len = r.u32()
    body = bytes(r.remaining())
    if len(body) != body_len:
        raise Truncated()
    return Envelope(codec_ver, msg_type, flags, body)
