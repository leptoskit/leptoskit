import { sensor } from "@kit.SensorServiceKit";
import {
	CANCELLED,
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
		return new Promise<ArrayBuffer>((resolve, reject) => {
			let settled = false;
			const cb = (data: sensor.PedometerResponse): void => {
				if (settled) return;
				settled = true;
				try {
					sensor.off(sensor.SensorId.PEDOMETER, cb);
				} catch {
					// Ignore cleanup errors.
				}
				const buf = new ArrayBuffer(32);
				const view = new DataView(buf);
				view.setUint32(0, 1, true); // BootCounter scope
				view.setUint32(4, 0, true);
				view.setBigUint64(8, BigInt(Math.max(0, Math.round(data.steps))), true);
				view.setFloat64(16, Date.now() / 1000.0, true);
				view.setFloat64(24, 0.0, true);
				resolve(buf);
			};
			call.onCancel(() => {
				if (!settled) {
					settled = true;
					try {
						sensor.off(sensor.SensorId.PEDOMETER, cb);
					} catch {
						// Ignore cleanup errors.
					}
					reject(new PluginError(CANCELLED));
				}
			});
			try {
				sensor.on(sensor.SensorId.PEDOMETER, cb);
			} catch {
				settled = true;
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
