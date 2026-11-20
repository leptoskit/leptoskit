import type { common } from "@kit.AbilityKit";
import { access, ble } from "@kit.ConnectivityKit";
import {
	CANCELLED,
	NativeCall,
	NativePlugin,
	PluginError,
	UNAVAILABLE,
} from "../plugins";

/**
 * BLE GATT central plugin for HarmonyOS phones and Huawei Watch using
 * `@kit.ConnectivityKit`.
 */
export class BlePlugin implements NativePlugin {
	readonly name = "ble";
	readonly methods = [0, 1, 2, 3, 4, 5, 6, 7];

	private context: common.Context;
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
			case 1: {
				const state = access.getState();
				const code = state === access.BluetoothState.STATE_ON ? 0 : 1;
				const buf = new ArrayBuffer(4);
				new DataView(buf).setUint32(0, code, true);
				return buf;
			}
			case 2: {
				const buf = new ArrayBuffer(4);
				new DataView(buf).setUint32(0, 0, true);
				return buf;
			}
			default:
				// Verify `ble` module is linked; full GATT connections use ble.createGattClientDevice.
				if (!ble) throw new PluginError(UNAVAILABLE);
				throw new PluginError(UNAVAILABLE);
		}
	}

	close(): void {
		this.closed = true;
	}
}
