import { PodError } from "./errors.js";

export class Reader {
	private remaining: Uint8Array;

	constructor(input: Uint8Array) {
		this.remaining = input;
	}

	getRemaining(): Uint8Array {
		return this.remaining;
	}

	private take(len: number): Uint8Array {
		if (this.remaining.length < len) {
			throw PodError.truncated();
		}
		const bytes = this.remaining.subarray(0, len);
		this.remaining = this.remaining.subarray(len);
		return bytes;
	}

	u8(): number {
		return this.take(1)[0];
	}

	u16(): number {
		const bytes = this.take(2);
		return new DataView(bytes.buffer, bytes.byteOffset, 2).getUint16(0, true);
	}

	u32(): number {
		const bytes = this.take(4);
		return new DataView(bytes.buffer, bytes.byteOffset, 4).getUint32(0, true);
	}

	u64(): bigint {
		const bytes = this.take(8);
		return new DataView(bytes.buffer, bytes.byteOffset, 8).getBigUint64(
			0,
			true,
		);
	}

	i8(): number {
		const v = this.u8();
		return (v << 24) >> 24;
	}

	i16(): number {
		const v = this.u16();
		return (v << 16) >> 16;
	}

	i32(): number {
		const v = this.u32();
		return v | 0;
	}

	i64(): bigint {
		return BigInt.asIntN(64, this.u64());
	}

	bool(): boolean {
		const tag = this.u8();
		if (tag === 0) return false;
		if (tag === 1) return true;
		throw PodError.invalidTag(tag);
	}

	bytes(): Uint8Array {
		const len = this.u32();
		return this.take(len);
	}

	string(): string {
		const raw = this.bytes();
		try {
			return new TextDecoder("utf-8", { fatal: true }).decode(raw);
		} catch {
			throw PodError.invalidUtf8();
		}
	}

	vecLen(): number {
		return this.u32();
	}

	optionTag(): boolean {
		const tag = this.u8();
		if (tag === 0) return false;
		if (tag === 1) return true;
		throw PodError.invalidTag(tag);
	}

	resultTag(): boolean {
		const tag = this.u8();
		if (tag === 0) return true;
		if (tag === 1) return false;
		throw PodError.invalidTag(tag);
	}

	structHeader(): [number, number] {
		return [this.u16(), this.u16()];
	}

	variant(): number {
		return this.u16();
	}

	finish(): void {
		if (this.remaining.length !== 0) {
			throw PodError.trailing();
		}
	}
}
