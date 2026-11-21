import type { common } from "@kit.AbilityKit";
import { sensor } from "@kit.SensorServiceKit";
import {
	CANCELLED,
	INVALID_ARGUMENT,
	NativeCall,
	NativePlugin,
	PluginError,
	UNAVAILABLE,
} from "../plugins";

const MAX_BATCH = 128;
const KIND_HR = 0;
const KIND_STEPS = 4;
const KIND_WEAR = 8;

/**
 * Direct on-watch biometric sensor plugin for Huawei Watch (HarmonyOS Wearable)
 * using `@kit.SensorServiceKit` (`HEART_RATE`, `PEDOMETER`, `WEAR_DETECTION`).
 */
export class WearablePlugin implements NativePlugin {
	readonly name = "wearable";
	readonly methods = [0, 1, 2, 3, 4, 5, 6, 7];

	private context: common.Context;
	private buffer: Uint8Array[] = [];
	private dropped = 0;
	private waiter:
		| { resolve: (data: ArrayBuffer) => void; reject: (err: PluginError) => void }
		| undefined = undefined;
	private passiveStops: (() => void)[] = [];
	private liveStops: (() => void)[] = [];
	private lastSteps: number | undefined = undefined;
	private lastStepsTime: number | undefined = undefined;
	private closed = false;

	constructor(context: common.Context) {
		this.context = context;
	}

	async invoke(call: NativeCall): Promise<ArrayBuffer> {
		if (this.closed) {
			throw new PluginError(CANCELLED);
		}
		switch (call.method) {
			case 0:
				return this.capabilities();
			case 1:
			case 2:
				return encodeU32(2);
			case 3:
				this.stopGroup(this.passiveStops);
				this.startSensors(readMask(call.payload), 5_000, this.passiveStops);
				return new ArrayBuffer(0);
			case 4:
				this.stopGroup(this.passiveStops);
				return new ArrayBuffer(0);
			case 5: {
				if (call.payload.byteLength !== 12) throw new PluginError(INVALID_ARGUMENT);
				const view = new DataView(call.payload);
				const mask = view.getUint32(0, true);
				const intervalMs = view.getUint32(4, true);
				if (mask === 0 || intervalMs < 200) throw new PluginError(INVALID_ARGUMENT);
				this.stopGroup(this.liveStops);
				this.startSensors(mask, intervalMs, this.liveStops);
				return new ArrayBuffer(0);
			}
			case 6:
				this.stopGroup(this.liveStops);
				return new ArrayBuffer(0);
			case 7:
				return this.nextBatch(call);
			default:
				throw new PluginError(UNAVAILABLE);
		}
	}

	close(): void {
		this.closed = true;
		this.stopGroup(this.passiveStops);
		this.stopGroup(this.liveStops);
		if (this.waiter !== undefined) {
			const pending = this.waiter;
			this.waiter = undefined;
			pending.reject(new PluginError(CANCELLED));
		}
	}

	private capabilities(): ArrayBuffer {
		// PlatformKind::HarmonyOs = 3; HR + steps + wear detection; passive + live + huawei_sensor_kit + off_body
		const buf = new ArrayBuffer(12);
		const view = new DataView(buf);
		view.setUint32(0, 3, true);
		view.setUint32(4, (1 << 0) | (1 << 4) | (1 << 8), true);
		view.setUint32(8, 1 | 2 | 8 | 16, true);
		return buf;
	}

	private startSensors(mask: number, intervalMs: number, group: (() => void)[]): void {
		const intervalNs = intervalMs * 1_000_000;
		if ((mask & (1 << 0)) !== 0) {
			const hrCb = (data: sensor.HeartRateResponse): void => {
				if (data.heartRate > 0) {
					this.enqueue(encodeHeartRate(Date.now(), data.heartRate, 4));
				}
			};
			try {
				sensor.on(sensor.SensorId.HEART_RATE, hrCb, { interval: intervalNs });
				group.push(() => sensor.off(sensor.SensorId.HEART_RATE, hrCb));
			} catch {
				// Sensor not present on non-wearable HarmonyOS device.
			}
		}
		if ((mask & (1 << 4)) !== 0) {
			const pedCb = (data: sensor.PedometerResponse): void => {
				const now = Date.now();
				const total = Math.max(0, Math.round(data.steps));
				if (this.lastSteps !== undefined && this.lastStepsTime !== undefined && total > this.lastSteps) {
					this.enqueue(encodeSteps(this.lastStepsTime, now, total - this.lastSteps));
				}
				this.lastSteps = total;
				this.lastStepsTime = now;
			};
			try {
				sensor.on(sensor.SensorId.PEDOMETER, pedCb, { interval: intervalNs });
				group.push(() => sensor.off(sensor.SensorId.PEDOMETER, pedCb));
			} catch {
				// Pedometer not present.
			}
		}
		if ((mask & (1 << 8)) !== 0) {
			const wearCb = (data: sensor.WearDetectionResponse): void => {
				this.enqueue(encodeWearState(Date.now(), data.value === 1));
			};
			try {
				sensor.on(sensor.SensorId.WEAR_DETECTION, wearCb, { interval: intervalNs });
				group.push(() => sensor.off(sensor.SensorId.WEAR_DETECTION, wearCb));
			} catch {
				// Wear detection not present.
			}
		}
	}

	private stopGroup(group: (() => void)[]): void {
		const stops = group.splice(0, group.length);
		for (const stop of stops) {
			try {
				stop();
			} catch {
				// Ignore cleanup errors.
			}
		}
	}

	private enqueue(reading: Uint8Array): void {
		if (this.buffer.length >= MAX_BATCH) {
			this.buffer.shift();
			this.dropped += 1;
		}
		this.buffer.push(reading);
		if (this.waiter !== undefined) {
			const pending = this.waiter;
			this.waiter = undefined;
			pending.resolve(this.drain());
		}
	}

	private nextBatch(call: NativeCall): Promise<ArrayBuffer> {
		if (this.buffer.length > 0 || this.dropped > 0) {
			return Promise.resolve(this.drain());
		}
		return new Promise<ArrayBuffer>((resolve, reject) => {
			call.onCancel(() => {
				this.waiter = undefined;
				reject(new PluginError(CANCELLED));
			});
			this.waiter = { resolve, reject };
		});
	}

	private drain(): ArrayBuffer {
		let total = 8;
		for (const item of this.buffer) total += item.byteLength;
		const out = new Uint8Array(total);
		const view = new DataView(out.buffer);
		view.setUint32(0, this.dropped, true);
		view.setUint32(4, this.buffer.length, true);
		let offset = 8;
		for (const item of this.buffer) {
			out.set(item, offset);
			offset += item.byteLength;
		}
		this.dropped = 0;
		this.buffer = [];
		return out.buffer;
	}
}

function readMask(payload: ArrayBuffer): number {
	if (payload.byteLength !== 4) throw new PluginError(INVALID_ARGUMENT);
	const mask = new DataView(payload).getUint32(0, true);
	if (mask === 0) throw new PluginError(INVALID_ARGUMENT);
	return mask;
}

function encodeU32(value: number): ArrayBuffer {
	const buf = new ArrayBuffer(4);
	new DataView(buf).setUint32(0, value >>> 0, true);
	return buf;
}

function encodeHeartRate(timeMs: number, bpm: number, accuracy: number): Uint8Array {
	const buf = new ArrayBuffer(24);
	const view = new DataView(buf);
	view.setUint32(0, KIND_HR, true);
	view.setBigInt64(4, BigInt(Math.trunc(timeMs)), true);
	view.setFloat64(12, bpm, true);
	view.setUint32(20, accuracy, true);
	return new Uint8Array(buf);
}

function encodeSteps(startMs: number, endMs: number, count: number): Uint8Array {
	const buf = new ArrayBuffer(28);
	const view = new DataView(buf);
	view.setUint32(0, KIND_STEPS, true);
	view.setBigInt64(4, BigInt(Math.trunc(startMs)), true);
	view.setBigInt64(12, BigInt(Math.trunc(endMs)), true);
	view.setBigInt64(20, BigInt(Math.trunc(count)), true);
	return new Uint8Array(buf);
}

function encodeWearState(timeMs: number, onWrist: boolean): Uint8Array {
	const buf = new ArrayBuffer(16);
	const view = new DataView(buf);
	view.setUint32(0, KIND_WEAR, true);
	view.setBigInt64(4, BigInt(Math.trunc(timeMs)), true);
	view.setUint32(12, onWrist ? 1 : 0, true);
	return new Uint8Array(buf);
}
