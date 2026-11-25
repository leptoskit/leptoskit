import type { common } from "@kit.AbilityKit";
import {
	CANCELLED,
	NativeCall,
	NativePlugin,
	PluginError,
	UNAVAILABLE,
} from "../plugins";

/**
 * Bidirectional HarmonyOS phone <-> Huawei Watch companion sync backed by
 * `@kit.WearEngine` when available on Huawei devices.
 */
export class WatchSyncPlugin implements NativePlugin {
	readonly name = "watch-sync";
	readonly methods = [0, 1, 2, 3, 4, 5];

	private context: common.Context;
	private closed = false;

	constructor(context: common.Context) {
		this.context = context;
	}

	async invoke(call: NativeCall): Promise<ArrayBuffer> {
		if (this.closed) {
			throw new PluginError(CANCELLED);
		}
		const wearEngine = this.loadWearEngine();
		switch (call.method) {
			case 0:
				return this.encodeCurrentStatus(wearEngine);
			case 4:
				return new ArrayBuffer(0);
			case 5: {
				const current = this.encodeCurrentStatus(wearEngine);
				if (!buffersEqual(current, call.payload)) {
					return current;
				}
				return new Promise<ArrayBuffer>((_resolve, reject) => {
					call.onCancel(() => reject(new PluginError(CANCELLED)));
				});
			}
			default:
				throw new PluginError(UNAVAILABLE);
		}
	}

	close(): void {
		this.closed = true;
	}

	private encodeCurrentStatus(
		wearEngine: Record<string, unknown> | undefined,
	): ArrayBuffer {
		const buf = new ArrayBuffer(12);
		const view = new DataView(buf);
		view.setUint32(0, wearEngine ? 3 : 0, true); // HuaweiWearEngine or None
		view.setUint32(4, wearEngine ? 1 : 0, true);
		view.setUint32(8, 0xffffffff, true);
		return buf;
	}

	private loadWearEngine(): Record<string, unknown> | undefined {
		try {
			const loader = (globalThis as Record<string, unknown>)["requireNapi"] as
				| ((name: string) => Record<string, unknown> | undefined)
				| undefined;
			return loader ? loader("wearEngine") : undefined;
		} catch {
			return undefined;
		}
	}
}

function buffersEqual(a: ArrayBuffer, b: ArrayBuffer): boolean {
	if (a.byteLength !== b.byteLength) return false;
	const ua = new Uint8Array(a);
	const ub = new Uint8Array(b);
	for (let i = 0; i < ua.byteLength; i++) {
		if (ua[i] !== ub[i]) return false;
	}
	return true;
}
