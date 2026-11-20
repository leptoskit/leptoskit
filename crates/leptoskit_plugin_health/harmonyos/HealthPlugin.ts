import type { common } from "@kit.AbilityKit";
import {
	CANCELLED,
	INVALID_ARGUMENT,
	NativeCall,
	NativePlugin,
	PluginError,
	UNAVAILABLE,
} from "../plugins";

// Wire codes shared with crates/leptoskit_plugin_health/src/{types,records}.rs.
const METHOD_AVAILABILITY = 0;
const METHOD_FEATURES = 1;
const METHOD_GRANTED = 2;
const METHOD_REQUEST = 3;
const METHOD_READ = 4;
const METHOD_CHANGES_TOKEN = 5;
const METHOD_CHANGES = 6;

const AVAILABLE = 0;
const UNSUPPORTED = 3;

const ABSENT_LENGTH = 0xffffffff;
const FLAG_EXACT = 4;

// HarmonyOS Health Service Kit covers all types except Apple-only SDNN (bit 3).
const HARMONY_TYPES = 0x1ff3;

interface HealthStoreSample {
	id: string;
	dataType: number;
	startMs: number;
	endMs: number;
	appId?: string;
	device?: string;
	value: number;
}

class WireWriter {
	private chunks: Uint8Array[] = [];
	private total = 0;

	u32(value: number): this {
		const buf = new ArrayBuffer(4);
		new DataView(buf).setUint32(0, value >>> 0, true);
		return this.push(new Uint8Array(buf));
	}

	i64(value: number): this {
		const buf = new ArrayBuffer(8);
		new DataView(buf).setBigInt64(0, BigInt(Math.trunc(value)), true);
		return this.push(new Uint8Array(buf));
	}

	f64(value: number): this {
		const buf = new ArrayBuffer(8);
		new DataView(buf).setFloat64(0, value, true);
		return this.push(new Uint8Array(buf));
	}

	optionalString(value: string | undefined): this {
		if (value === undefined) {
			return this.u32(ABSENT_LENGTH);
		}
		return this.string(value);
	}

	string(value: string): this {
		const encoded = encodeUtf8(value);
		this.u32(encoded.byteLength);
		return this.push(encoded);
	}

	private push(bytes: Uint8Array): this {
		this.chunks.push(bytes);
		this.total += bytes.byteLength;
		return this;
	}

	bytes(): ArrayBuffer {
		const out = new Uint8Array(this.total);
		let offset = 0;
		for (const chunk of this.chunks) {
			out.set(chunk, offset);
			offset += chunk.byteLength;
		}
		return out.buffer;
	}
}

function encodeUtf8(text: string): Uint8Array {
	const bytes: number[] = [];
	for (let i = 0; i < text.length; i++) {
		const code = text.charCodeAt(i);
		if (code < 0x80) {
			bytes.push(code);
		} else if (code < 0x800) {
			bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
		} else {
			bytes.push(
				0xe0 | (code >> 12),
				0x80 | ((code >> 6) & 0x3f),
				0x80 | (code & 0x3f),
			);
		}
	}
	return new Uint8Array(bytes);
}

/**
 * Reads Huawei Health Service Kit records on HarmonyOS phones and watches
 * (Huawei Watch 3/4/5/Ultimate). Falls back gracefully to `Unsupported` on
 * OpenHarmony devices without `@kit.HealthServiceKit`.
 */
export class HealthPlugin implements NativePlugin {
	readonly name = "health";
	readonly methods = [
		METHOD_AVAILABILITY,
		METHOD_FEATURES,
		METHOD_GRANTED,
		METHOD_REQUEST,
		METHOD_READ,
		METHOD_CHANGES_TOKEN,
		METHOD_CHANGES,
	];

	private context: common.Context;
	private grantedMask = 0;
	private closed = false;

	constructor(context: common.Context) {
		this.context = context;
	}

	async invoke(call: NativeCall): Promise<ArrayBuffer> {
		if (this.closed) {
			throw new PluginError(CANCELLED);
		}

		const store = this.loadHealthStore();
		switch (call.method) {
			case METHOD_AVAILABILITY:
				return new WireWriter().u32(store ? AVAILABLE : UNSUPPORTED).bytes();
			case METHOD_FEATURES:
				if (!store) throw new PluginError(UNAVAILABLE);
				return new WireWriter().u32(HARMONY_TYPES).u32(0).bytes();
			case METHOD_GRANTED:
				if (!store) throw new PluginError(UNAVAILABLE);
				return new WireWriter().u32(this.grantedMask).u32(FLAG_EXACT).bytes();
			case METHOD_REQUEST:
				return this.requestPermissions(call.payload, store);
			case METHOD_READ:
				return this.readSamples(call.payload, store);
			case METHOD_CHANGES_TOKEN:
				if (!store) throw new PluginError(UNAVAILABLE);
				return new WireWriter().string(`harmony:${Date.now()}`).bytes();
			case METHOD_CHANGES:
				if (!store) throw new PluginError(UNAVAILABLE);
				return new WireWriter()
					.u32(0)
					.u32(0)
					.optionalString(`harmony:${Date.now()}`)
					.u32(0)
					.bytes();
			default:
				throw new PluginError(UNAVAILABLE);
		}
	}

	close(): void {
		this.closed = true;
	}

	private loadHealthStore(): Record<string, unknown> | undefined {
		try {
			const loader = (globalThis as Record<string, unknown>)["requireNapi"] as
				| ((name: string) => Record<string, unknown> | undefined)
				| undefined;
			return loader ? loader("health.healthStore") : undefined;
		} catch {
			return undefined;
		}
	}

	private async requestPermissions(
		payload: ArrayBuffer,
		store: Record<string, unknown> | undefined,
	): Promise<ArrayBuffer> {
		if (!store) throw new PluginError(UNAVAILABLE);
		if (payload.byteLength !== 8) throw new PluginError(INVALID_ARGUMENT);
		const view = new DataView(payload);
		const mask = view.getUint32(0, true) & HARMONY_TYPES;
		if (mask === 0) throw new PluginError(INVALID_ARGUMENT);

		const requestAuth = store["requestAuthorizations"] as
			| ((ctx: common.Context, req: Record<string, unknown>) => Promise<void>)
			| undefined;
		if (requestAuth) {
			await requestAuth(this.context, { readDataTypes: [mask] });
		}
		this.grantedMask |= mask;
		return new WireWriter().u32(this.grantedMask).u32(FLAG_EXACT).bytes();
	}

	private async readSamples(
		payload: ArrayBuffer,
		store: Record<string, unknown> | undefined,
	): Promise<ArrayBuffer> {
		if (!store) throw new PluginError(UNAVAILABLE);
		if (payload.byteLength < 28) throw new PluginError(INVALID_ARGUMENT);
		const view = new DataView(payload);
		const dataType = view.getUint32(0, true);
		const startMs = Number(view.getBigInt64(4, true));
		const endMs = Number(view.getBigInt64(12, true));
		const pageSize = view.getUint32(20, true);
		if (startMs >= endMs || pageSize < 1 || pageSize > 1000) {
			throw new PluginError(INVALID_ARGUMENT);
		}

		const reader = store["readData"] as
			| ((req: Record<string, unknown>) => Promise<HealthStoreSample[]>)
			| undefined;
		const rows: HealthStoreSample[] = reader
			? await reader({ dataType, startTime: startMs, endTime: endMs, limit: pageSize })
			: [];

		const writer = new WireWriter();
		writer.u32(rows.length);
		for (const row of rows) {
			writer
				.string(row.id)
				.u32(dataType)
				.i64(row.startMs)
				.i64(row.endMs)
				.optionalString(row.appId)
				.optionalString("Huawei Health")
				.optionalString(row.device ?? "Huawei Watch");
			if (dataType === 0) {
				writer.u32(1).i64(row.endMs).f64(row.value);
			} else if (dataType === 4) {
				writer.i64(Math.max(0, Math.round(row.value)));
			} else {
				writer.f64(row.value);
			}
		}
		writer.optionalString(undefined);
		return writer.bytes();
	}
}
