import { batteryInfo } from "@kit.BasicServicesKit";
import {
	CANCELLED,
	INVALID_ARGUMENT,
	NativeCall,
	NativePlugin,
	PluginError,
	UNAVAILABLE,
} from "../plugins";

/**
 * Battery charge and charging state for HarmonyOS phones and Huawei Watch
 * via `@kit.BasicServicesKit`.
 */
export class BatteryPlugin implements NativePlugin {
	readonly name = "battery";
	readonly methods = [0, 1, 2];
	private closed = false;

	async invoke(call: NativeCall): Promise<ArrayBuffer> {
		if (this.closed) {
			throw new PluginError(CANCELLED);
		}
		if (call.method === 0) {
			if (call.payload.byteLength !== 0) {
				throw new PluginError(INVALID_ARGUMENT);
			}
			return this.snapshot();
		}
		throw new PluginError(UNAVAILABLE);
	}

	close(): void {
		this.closed = true;
	}

	private snapshot(): ArrayBuffer {
		const soc = batteryInfo.batterySOC;
		const charge = soc >= 0 && soc <= 100 ? Math.round(soc * 10) : 0xffffffff;
		const status = batteryInfo.chargingStatus;
		let state = 0; // Unknown
		if (status === batteryInfo.BatteryChargeState.DISABLE) state = 1; // Discharging
		else if (status === batteryInfo.BatteryChargeState.ENABLE) state = 2; // Charging
		else if (status === batteryInfo.BatteryChargeState.FULL) state = 3; // Full

		const out = new ArrayBuffer(16);
		const view = new DataView(out);
		view.setUint32(0, 2, true); // present = true
		view.setUint32(4, charge, true);
		view.setUint32(8, state, true);
		view.setUint32(12, 0, true); // power_save = unknown
		return out;
	}
}
