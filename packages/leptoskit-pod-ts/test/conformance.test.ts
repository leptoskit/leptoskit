import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
	decodeStructVersion,
	encodeStructV0,
	MessageType,
	PodError,
	Reader,
	unwrap,
	wrap,
	Writer,
} from "../src/index.ts";

const vectorsPath = join(
	dirname(fileURLToPath(import.meta.url)),
	"../../../crates/leptoskit_pod/conformance/vectors.json",
);

interface Vector {
	name: string;
	hex: string;
	envelope?: boolean;
}

interface Malformed {
	name: string;
	hex: string;
	expect: string;
	envelope?: boolean;
}

interface Corpus {
	vectors: Vector[];
	malformed: Malformed[];
}

function hexEncode(bytes: Uint8Array): string {
	return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function hexDecode(hex: string): Uint8Array {
	const out = new Uint8Array(hex.length / 2);
	for (let i = 0; i < hex.length; i += 2) {
		out[i / 2] = parseInt(hex.slice(i, i + 2), 16);
	}
	return out;
}

function encodeNote(
	priority: number,
	archived: boolean,
	title: string,
): Uint8Array {
	return encodeStructV0((w) => {
		w.u8(priority);
		w.bool(archived);
		w.string(title);
	});
}

function decodeNote(
	r: Reader,
): { priority: number; archived: boolean; title: string } {
	return decodeStructVersion(r, 0, (inner) => ({
		priority: inner.u8(),
		archived: inner.bool(),
		title: inner.string(),
	}));
}

function encodeLiveUpdate(
	kind: "Ok" | "Patch",
	priority?: number,
	title?: string,
): Uint8Array {
	const w = Writer.new();
	if (kind === "Ok") {
		w.variant(0);
		return w.finish();
	}
	w.variant(1);
	w.structHeader(0, 0);
	w.u8(priority!);
	w.string(title!);
	return w.finish();
}

function encodeVector(name: string): Uint8Array {
	switch (name) {
		case "empty_struct_v0":
			return encodeStructV0(() => {});
		case "note_hi":
			return encodeNote(7, true, "hi");
		case "option_none_u8": {
			const w = Writer.new();
			w.optionTag(false);
			return w.finish();
		}
		case "option_some_u8_9": {
			const w = Writer.new();
			w.optionTag(true);
			w.u8(9);
			return w.finish();
		}
		case "result_ok_u8_3": {
			const w = Writer.new();
			w.resultTag(true);
			w.u8(3);
			return w.finish();
		}
		case "result_err_u8_4": {
			const w = Writer.new();
			w.resultTag(false);
			w.u8(4);
			return w.finish();
		}
		case "vec_u8_empty": {
			const w = Writer.new();
			w.vecLen(0);
			return w.finish();
		}
		case "vec_u8_1_2_3": {
			const w = Writer.new();
			w.vecLen(3);
			w.u8(1);
			w.u8(2);
			w.u8(3);
			return w.finish();
		}
		case "enum_unit_variant_0":
			return encodeLiveUpdate("Ok");
		case "enum_struct_variant":
			return encodeLiveUpdate("Patch", 7, "abc");
		case "envelope_rpc_empty":
			return wrap(MessageType.Rpc, 0, new Uint8Array(0));
		default:
			throw new Error(`no encoder for ${name}`);
	}
}

function decodeVector(name: string, bytes: Uint8Array): void {
	const r = new Reader(bytes);
	switch (name) {
		case "empty_struct_v0": {
			const [v, f] = r.structHeader();
			assert.equal(v, 0);
			assert.equal(f, 0);
			r.finish();
			break;
		}
		case "note_hi":
			decodeNote(r);
			r.finish();
			break;
		case "option_none_u8":
			assert.equal(r.optionTag(), false);
			r.finish();
			break;
		case "option_some_u8_9":
			assert.equal(r.optionTag(), true);
			assert.equal(r.u8(), 9);
			r.finish();
			break;
		case "result_ok_u8_3":
			assert.equal(r.resultTag(), true);
			assert.equal(r.u8(), 3);
			r.finish();
			break;
		case "result_err_u8_4":
			assert.equal(r.resultTag(), false);
			assert.equal(r.u8(), 4);
			r.finish();
			break;
		case "vec_u8_empty":
			assert.equal(r.vecLen(), 0);
			r.finish();
			break;
		case "vec_u8_1_2_3": {
			const n = r.vecLen();
			assert.equal(n, 3);
			assert.deepEqual([r.u8(), r.u8(), r.u8()], [1, 2, 3]);
			r.finish();
			break;
		}
		case "enum_unit_variant_0":
			assert.equal(r.variant(), 0);
			r.finish();
			break;
		case "enum_struct_variant": {
			assert.equal(r.variant(), 1);
			decodeStructVersion(r, 0, (inner) => ({
				priority: inner.u8(),
				title: inner.string(),
			}));
			r.finish();
			break;
		}
		case "envelope_rpc_empty": {
			unwrap(bytes);
			return;
		}
		default:
			throw new Error(`no decoder for ${name}`);
	}
}

const corpus: Corpus = JSON.parse(readFileSync(vectorsPath, "utf8"));

test("conformance vectors encode/decode", () => {
	for (const v of corpus.vectors) {
		const bytes = hexDecode(v.hex);
		const encoded = encodeVector(v.name);
		assert.equal(hexEncode(encoded), v.hex, `${v.name} encode`);
		decodeVector(v.name, bytes);
	}
});

test("conformance malformed", () => {
	for (const m of corpus.malformed) {
		const bytes = hexDecode(m.hex);
		let err: PodError | undefined;
		try {
			if (m.envelope) {
				unwrap(bytes);
			} else if (m.name.includes("utf8")) {
				new Reader(bytes).string();
			} else if (m.name.includes("bool")) {
				new Reader(bytes).bool();
			} else {
				new Reader(bytes).u16();
			}
		} catch (e) {
			if (e instanceof PodError) {
				err = e;
			} else {
				throw e;
			}
		}
		assert.ok(err, `${m.name} should throw`);
		assert.equal(err!.code, m.expect, `${m.name} error code`);
	}
});
