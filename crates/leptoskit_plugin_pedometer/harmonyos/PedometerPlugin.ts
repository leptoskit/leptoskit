import { sensor } from "@kit.SensorServiceKit";
import {
	CANCELLED,
	INVALID_ARGUMENT,
	NativeCall,
	NativePlugin,
	PluginError,
	UNAVAILABLE,
} from "../plugins";

/**
 * Step counter pedometer for HarmonyOS phones and Huawei Watch via
 * `@kit.SensorServiceKit` (`PEDOMETER`).
 */
export class PedometerPlugin implements NativePlugin {
	readonly name = "pedometer";
	readonly methods = [0, 1, 2, 3, 4];
	private closed = false;

	async invoke(call: NativeCall): Promise<ArrayBuffer> {
		if (this.closed) throw new PluginError(CANCELLED);
		switch (call.method) {
			case 0:
				return encodeU32(1);
			case 1:
			case 2:
				return encodeU32(3); // Granted
			case 3:
				return this.nextReading(call);
			default:
				throw new PluginError(UNAVAILABLE);
		}
	}

	close(): void {
		this.closed = true;
	}

	private nextReading(call: NativeCall): Promise<ArrayBuffer> {
		if (call.payload.byteLength !== 4) {
			return Promise.reject(new PluginError(INVALID_ARGUMENT));
		}
		const timeoutMs = new DataView(call.payload).getUint32(0, true);
		return new Promise<ArrayBuffer>((resolve, reject) => {
			let settled = false;
			let timer: ReturnType<typeof setTimeout> | undefined = undefined;
			const cleanup = (cb: (data: sensor.PedometerResponse) => void): void => {
				if (timer !== undefined) {
					clearTimeout(timer);
					timer = undefined;
				}
				try {
					sensor.off(sensor.SensorId.PEDOMETER, cb);
				} catch {
					// Ignore cleanup errors.
				}
			};
			const cb = (data: sensor.PedometerResponse): void => {
				if (settled) return;
				settled = true;
				cleanup(cb);
				// bounded_reply framing: 4-byte status (0 = reading) + 32-byte Reading.
				const buf = new ArrayBuffer(36);
				const view = new DataView(buf);
				view.setUint32(0, 0, true); // bounded_reply: Present
				view.setUint32(4, 1, true); // BootCounter scope
				view.setUint32(8, 0, true); // reserved
				view.setBigUint64(
					12,
					BigInt(Math.max(0, Math.round(data.steps))),
					true,
				);
				view.setFloat64(
					20,
					data.timestamp > 0 ? data.timestamp / 1e9 : Date.now() / 1000.0,
					true,
				);
				view.setFloat64(28, 0.0, true);
				resolve(buf);
			};
			if (timeoutMs > 0) {
				timer = setTimeout(() => {
					if (settled) return;
					settled = true;
					cleanup(cb);
					resolve(encodeU32(1)); // bounded_reply: TimedOut
				}, timeoutMs);
			}
			call.onCancel(() => {
				if (!settled) {
					settled = true;
					cleanup(cb);
					reject(new PluginError(CANCELLED));
				}
			});
			try {
				sensor.on(sensor.SensorId.PEDOMETER, cb);
			} catch {
				settled = true;
				cleanup(cb);
				reject(new PluginError(UNAVAILABLE));
			}
		});
	}
}

function encodeU32(value: number): ArrayBuffer {
	const buf = new ArrayBuffer(4);
	new DataView(buf).setUint32(0, value >>> 0, true);
	return buf;
}
