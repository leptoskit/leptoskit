/// Appends little-endian Pod fields into a growable buffer.
public struct Writer: Sendable {
	private var bytes: [UInt8] = []

	public init() {}

	public init(capacity: Int) {
		bytes.reserveCapacity(capacity)
	}

	public var count: Int { bytes.count }

	public var isEmpty: Bool { bytes.isEmpty }

	public mutating func raw(_ value: [UInt8]) {
		bytes.append(contentsOf: value)
	}

	public mutating func raw(_ value: some Collection<UInt8>) {
		bytes.append(contentsOf: value)
	}

	public mutating func u8(_ value: UInt8) {
		bytes.append(value)
	}

	public mutating func u16(_ value: UInt16) {
		bytes.append(contentsOf: value.littleEndianBytes)
	}

	public mutating func u32(_ value: UInt32) {
		bytes.append(contentsOf: value.littleEndianBytes)
	}

	public mutating func u64(_ value: UInt64) {
		bytes.append(contentsOf: value.littleEndianBytes)
	}

	public mutating func i8(_ value: Int8) { u8(UInt8(bitPattern: value)) }
	public mutating func i16(_ value: Int16) { u16(UInt16(bitPattern: value)) }
	public mutating func i32(_ value: Int32) { u32(UInt32(bitPattern: value)) }
	public mutating func i64(_ value: Int64) { u64(UInt64(bitPattern: value)) }

	public mutating func bool(_ value: Bool) {
		u8(value ? 1 : 0)
	}

	public mutating func bytes(_ value: [UInt8]) throws {
		guard value.count <= UInt32.max else { throw PodError.tooLarge }
		u32(UInt32(value.count))
		bytes.append(contentsOf: value)
	}

	public mutating func string(_ value: String) throws {
		try bytes(Array(value.utf8))
	}

	public mutating func vecLen(_ len: Int) throws {
		guard len <= Int(UInt32.max) else { throw PodError.tooLarge }
		u32(UInt32(len))
	}

	public mutating func optionTag(some: Bool) {
		u8(some ? 1 : 0)
	}

	/// Result tag: `0` = ok, `1` = err (matches Rust `result_tag(!ok)`).
	public mutating func resultTag(ok: Bool) {
		u8(ok ? 0 : 1)
	}

	public mutating func structHeader(version: UInt8, flags: UInt8) {
		u8(version)
		u8(flags)
	}

	public mutating func variant(_ disc: UInt16) {
		u16(disc)
	}

	public func asSlice() -> [UInt8] { bytes }

	public mutating func finish() -> [UInt8] {
		let out = bytes
		bytes = []
		return out
	}
}

private extension FixedWidthInteger {
	var littleEndianBytes: [UInt8] {
		withUnsafeBytes(of: littleEndian) { Array($0) }
	}
}
