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
const HARMONY_TYPES = 0x1ff7;

interface HealthSleepStage {
	startMs: number;
	endMs: number;
	stage: number;
}

interface HealthStoreSample {
	id: string;
	dataType: number;
	startMs: number;
	endMs: number;
	appId?: string;
	device?: string;
	value: number;
	stages?: HealthSleepStage[];
	activity?: number;
	platformCode?: number;
	title?: string;
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
		let code = text.charCodeAt(i);
		if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length) {
			const low = text.charCodeAt(i + 1);
			if (low >= 0xdc00 && low <= 0xdfff) {
				code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
				i += 1;
			}
		}
		if (code < 0x80) {
			bytes.push(code);
		} else if (code < 0x800) {
			bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
		} else if (code < 0x10000) {
			bytes.push(
				0xe0 | (code >> 12),
				0x80 | ((code >> 6) & 0x3f),
				0x80 | (code & 0x3f),
			);
		} else {
			bytes.push(
				0xf0 | (code >> 18),
				0x80 | ((code >> 12) & 0x3f),
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
				return this.changesToken(call.payload, store);
			case METHOD_CHANGES:
				return this.readChanges(call.payload, store);
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
		const flags = view.getUint32(4, true);
		if (mask === 0 || (flags & ~0b11) !== 0) {
			throw new PluginError(INVALID_ARGUMENT);
		}

		const requestAuth = store["requestAuthorizations"] as
			| ((ctx: common.Context, req: Record<string, unknown>) => Promise<void>)
			| undefined;
		if (requestAuth) {
			const dataTypes: number[] = [];
			for (let bit = 0; bit <= 12; bit++) {
				if ((mask & (1 << bit)) !== 0) dataTypes.push(bit);
			}
			await requestAuth(this.context, { readDataTypes: dataTypes });
		}
		this.grantedMask |= mask;
		return new WireWriter().u32(this.grantedMask).u32(FLAG_EXACT).bytes();
	}

	private changesToken(
		payload: ArrayBuffer,
		store: Record<string, unknown> | undefined,
	): ArrayBuffer {
		if (!store) throw new PluginError(UNAVAILABLE);
		if (payload.byteLength !== 8) throw new PluginError(INVALID_ARGUMENT);
		const view = new DataView(payload);
		const mask = view.getUint32(0, true) & HARMONY_TYPES;
		const flags = view.getUint32(4, true);
		if (mask === 0 || (flags & ~0b11) !== 0) {
			throw new PluginError(INVALID_ARGUMENT);
		}
		return new WireWriter().string(`harmony:${mask}:${Date.now()}`).bytes();
	}

	private async readChanges(
		payload: ArrayBuffer,
		store: Record<string, unknown> | undefined,
	): Promise<ArrayBuffer> {
		if (!store) throw new PluginError(UNAVAILABLE);
		if (payload.byteLength < 4) throw new PluginError(INVALID_ARGUMENT);
		const view = new DataView(payload);
		const len = view.getUint32(0, true);
		if (len === 0 || payload.byteLength !== 4 + len) {
			throw new PluginError(INVALID_ARGUMENT);
		}
		const tokenBytes = new Uint8Array(payload, 4, len);
		const token = decodeAscii(tokenBytes);
		const parts = token.split(":");
		if (parts.length !== 3 || parts[0] !== "harmony") {
			throw new PluginError(INVALID_ARGUMENT);
		}
		const mask = Number(parts[1]);
		const sinceMs = Number(parts[2]);
		if (
			!Number.isFinite(mask) || !Number.isFinite(sinceMs) ||
			(mask & HARMONY_TYPES) === 0
		) {
			throw new PluginError(INVALID_ARGUMENT);
		}
		const now = Date.now();
		const upserted: HealthStoreSample[] = [];
		const reader = store["readData"] as
			| ((req: Record<string, unknown>) => Promise<HealthStoreSample[]>)
			| undefined;
		if (reader && now > sinceMs) {
			for (let bit = 0; bit <= 12; bit++) {
				if ((mask & (1 << bit)) === 0) continue;
				const rows = await reader({
					dataType: bit,
					startTime: sinceMs,
					endTime: now,
					limit: 200,
				});
				for (const row of rows) {
					upserted.push({ ...row, dataType: bit });
				}
			}
		}
		const writer = new WireWriter();
		writer.u32(upserted.length);
		for (const row of upserted) {
			encodeSample(writer, row, row.dataType);
		}
		writer.u32(0); // deleted count
		writer.optionalString(`harmony:${mask}:${now}`);
		writer.u32(0); // flags
		return writer.bytes();
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
		const tokenLen = view.getUint32(24, true);
		if (
			dataType > 12 ||
			(HARMONY_TYPES & (1 << dataType)) === 0 ||
			startMs >= endMs ||
			pageSize < 1 ||
			pageSize > 1000
		) {
			throw new PluginError(INVALID_ARGUMENT);
		}
		let pageToken: string | undefined = undefined;
		if (tokenLen === ABSENT_LENGTH) {
			if (payload.byteLength !== 28) throw new PluginError(INVALID_ARGUMENT);
		} else {
			if (tokenLen === 0 || payload.byteLength !== 28 + tokenLen) {
				throw new PluginError(INVALID_ARGUMENT);
			}
			pageToken = decodeAscii(new Uint8Array(payload, 28, tokenLen));
		}

		const reader = store["readData"] as
			| ((req: Record<string, unknown>) => Promise<HealthStoreSample[]>)
			| undefined;
		const rows: HealthStoreSample[] = reader
			? await reader({
				dataType,
				startTime: startMs,
				endTime: endMs,
				limit: pageSize,
				pageToken,
			})
			: [];

		const writer = new WireWriter();
		writer.u32(rows.length);
		for (const row of rows) {
			encodeSample(writer, row, dataType);
		}
		writer.optionalString(undefined);
		return writer.bytes();
	}
}

function encodeSample(
	writer: WireWriter,
	row: HealthStoreSample,
	dataType: number,
): void {
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
	} else if (dataType === 5) {
		const stages = row.stages ?? [];
		writer.u32(stages.length);
		for (const stage of stages) {
			writer.i64(stage.startMs).i64(stage.endMs).u32(stage.stage);
		}
	} else if (dataType === 8) {
		writer
			.u32(row.activity ?? 0)
			.u32(row.platformCode ?? 0)
			.optionalString(row.title);
	} else {
		writer.f64(row.value);
	}
}

function decodeAscii(bytes: Uint8Array): string {
	let out = "";
	for (let i = 0; i < bytes.byteLength; i++) {
		out += String.fromCharCode(bytes[i]);
	}
	return out;
}
