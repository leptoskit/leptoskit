from __future__ import annotations

import json
from pathlib import Path

import pytest

from leptoskit_pod import (
    MessageType,
    PodError,
    Reader,
    Writer,
    decode_struct_version,
    encode_struct_v0,
    unwrap,
    wrap,
)

VECTORS = (
    Path(__file__).resolve().parents[3]
    / "crates"
    / "leptoskit_pod"
    / "conformance"
    / "vectors.json"
)


def hex_encode(data: bytes) -> str:
    return data.hex()


def hex_decode(s: str) -> bytes:
    return bytes.fromhex(s)


def encode_note(priority: int, archived: bool, title: str) -> bytes:
    def fields(w: Writer) -> None:
        w.u8(priority)
        w.bool(archived)
        w.string(title)

    return encode_struct_v0(fields)


def decode_note(r: Reader) -> None:
    def fields(inner: Reader) -> None:
        inner.u8()
        inner.bool()
        inner.string()

    decode_struct_version(r, 0, fields)


def encode_vector(name: str) -> bytes:
    if name == "empty_struct_v0":
        return encode_struct_v0(lambda w: None)
    if name == "note_hi":
        return encode_note(7, True, "hi")
    if name == "option_none_u8":
        w = Writer.new()
        w.option_tag(False)
        return w.finish()
    if name == "option_some_u8_9":
        w = Writer.new()
        w.option_tag(True)
        w.u8(9)
        return w.finish()
    if name == "result_ok_u8_3":
        w = Writer.new()
        w.result_tag(True)
        w.u8(3)
        return w.finish()
    if name == "result_err_u8_4":
        w = Writer.new()
        w.result_tag(False)
        w.u8(4)
        return w.finish()
    if name == "vec_u8_empty":
        w = Writer.new()
        w.vec_len(0)
        return w.finish()
    if name == "vec_u8_1_2_3":
        w = Writer.new()
        w.vec_len(3)
        w.u8(1)
        w.u8(2)
        w.u8(3)
        return w.finish()
    if name == "enum_unit_variant_0":
        w = Writer.new()
        w.variant(0)
        return w.finish()
    if name == "enum_struct_variant":
        w = Writer.new()
        w.variant(1)
        w.struct_header(0)
        w.u8(7)
        w.string("abc")
        return w.finish()
    if name == "envelope_rpc_empty":
        return wrap(MessageType.Rpc, 0, b"")
    raise AssertionError(f"no encoder for {name}")


def decode_vector(name: str, data: bytes) -> None:
    if name == "envelope_rpc_empty":
        unwrap(data)
        return
    r = Reader(data)
    if name == "empty_struct_v0":
        assert r.struct_header() == 0
    elif name == "note_hi":
        decode_note(r)
    elif name == "option_none_u8":
        assert r.option_tag() is False
    elif name == "option_some_u8_9":
        assert r.option_tag() is True
        assert r.u8() == 9
    elif name == "result_ok_u8_3":
        assert r.result_tag() is True
        assert r.u8() == 3
    elif name == "result_err_u8_4":
        assert r.result_tag() is False
        assert r.u8() == 4
    elif name == "vec_u8_empty":
        assert r.vec_len() == 0
    elif name == "vec_u8_1_2_3":
        assert r.vec_len() == 3
        assert [r.u8(), r.u8(), r.u8()] == [1, 2, 3]
    elif name == "enum_unit_variant_0":
        assert r.variant() == 0
    elif name == "enum_struct_variant":
        assert r.variant() == 1

        def fields(inner: Reader) -> None:
            inner.u8()
            inner.string()

        decode_struct_version(r, 0, fields)
    else:
        raise AssertionError(f"no decoder for {name}")
    r.finish()


@pytest.fixture(scope="module")
def corpus() -> dict:
    return json.loads(VECTORS.read_text())


def test_conformance_vectors(corpus: dict) -> None:
    for v in corpus["vectors"]:
        data = hex_decode(v["hex"])
        encoded = encode_vector(v["name"])
        assert hex_encode(encoded) == v["hex"], v["name"]
        decode_vector(v["name"], data)


def test_conformance_malformed(corpus: dict) -> None:
    for m in corpus["malformed"]:
        data = hex_decode(m["hex"])
        with pytest.raises(PodError) as exc:
            name = m["name"]
            if m.get("envelope"):
                unwrap(data)
            elif "utf8" in name:
                Reader(data).string()
            elif "bool" in name or "option" in name or "result" in name:
                if "option" in name:
                    Reader(data).option_tag()
                elif "result" in name:
                    Reader(data).result_tag()
                else:
                    Reader(data).bool()
            elif "huge_len" in name or "vec_u8" in name:
                Reader(data).bytes()
            else:
                Reader(data).u16()
        assert exc.value.__class__.__name__ == m["expect"], m["name"]
