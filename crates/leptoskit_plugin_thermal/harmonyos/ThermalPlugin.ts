import { thermal } from "@kit.BasicServicesKit";
import {
	CANCELLED,
	INVALID_ARGUMENT,
	NativeCall,
	NativePlugin,
	PluginError,
	UNAVAILABLE,
} from "../plugins";

/**
 * Device thermal state for HarmonyOS phones and Huawei Watch via
 * `@kit.BasicServicesKit`.
 */
export class ThermalPlugin implements NativePlugin {
	readonly name = "thermal";
	readonly methods = [0, 1];
	private closed = false;

	async invoke(call: NativeCall): Promise<ArrayBuffer> {
		if (this.closed) {
			throw new PluginError(CANCELLED);
		}
		if (call.method === 0) {
			if (call.payload.byteLength !== 0) throw new PluginError(INVALID_ARGUMENT);
			return this.snapshot();
		}
		throw new PluginError(UNAVAILABLE);
	}

	close(): void {
		this.closed = true;
	}

	private snapshot(): ArrayBuffer {
		const level = thermal.getLevel();
		// Map HarmonyOS ThermalLevel (COOL..ESCAPE) to shared 4-level State.
		let state = 1; // Nominal
		if (level === thermal.ThermalLevel.WARM || level === thermal.ThermalLevel.HOT) {
			state = 2; // Fair
		} else if (level === thermal.ThermalLevel.OVERHEATED) {
			state = 3; // Serious
		} else if (level >= thermal.ThermalLevel.WARNING) {
			state = 4; // Critical
		}
		const out = new ArrayBuffer(8);
		const view = new DataView(out);
		view.setUint32(0, state, true);
		view.setUint32(4, 0xffffffff, true); // no Android status
		return out;
	}
}
