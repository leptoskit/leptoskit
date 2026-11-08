import { PodError } from "./errors.js";

export class Writer {
	private buf: number[] = [];

	static new(): Writer {
		return new Writer();
	}

	static withCapacity(capacity: number): Writer {
		const w = new Writer();
		w.buf = [];
		w.buf.length = 0;
		void capacity;
		return w;
	}

	len(): number {
		return this.buf.length;
	}

	isEmpty(): boolean {
		return this.buf.length === 0;
	}

	raw(value: Uint8Array | readonly number[]): void {
		for (const b of value) {
			this.buf.push(b);
		}
	}

	u8(value: number): void {
		this.buf.push(value & 0xff);
	}

	u16(value: number): void {
		const buf = new ArrayBuffer(2);
		new DataView(buf).setUint16(0, value, true);
		this.raw(new Uint8Array(buf));
	}

	u32(value: number): void {
		const buf = new ArrayBuffer(4);
		new DataView(buf).setUint32(0, value >>> 0, true);
		this.raw(new Uint8Array(buf));
	}

	u64(value: bigint | number): void {
		const buf = new ArrayBuffer(8);
		new DataView(buf).setBigUint64(0, BigInt(value), true);
		this.raw(new Uint8Array(buf));
	}

	i8(value: number): void {
		this.u8(value);
	}

	i16(value: number): void {
		this.u16(value);
	}

	i32(value: number): void {
		this.u32(value);
	}

	i64(value: bigint | number): void {
		this.u64(value);
	}

	bool(value: boolean): void {
		this.u8(value ? 1 : 0);
	}

	writeBytes(value: Uint8Array): void {
		if (value.length > 0xffffffff) {
			throw PodError.tooLarge();
		}
		this.u32(value.length);
		this.raw(value);
	}

	/** @alias writeBytes — matches Rust `Writer::bytes`. */
	bytes(value: Uint8Array): void {
		this.writeBytes(value);
	}

	string(value: string): void {
		this.bytes(new TextEncoder().encode(value));
	}

	vecLen(len: number): void {
		if (len > 0xffffffff) {
			throw PodError.tooLarge();
		}
		this.u32(len);
	}

	optionTag(some: boolean): void {
		this.u8(some ? 1 : 0);
	}

	resultTag(ok: boolean): void {
		this.u8(ok ? 0 : 1);
	}

	structHeader(version: number): void {
		this.u16(version);
	}

	variant(disc: number): void {
		this.u16(disc);
	}

	asSlice(): Uint8Array {
		return Uint8Array.from(this.buf);
	}

	finish(): Uint8Array {
		return this.asSlice();
	}
}
