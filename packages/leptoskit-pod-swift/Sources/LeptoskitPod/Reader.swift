/// Allocation-free Pod decoder over a borrowed buffer.
public struct Reader: Sendable {
	private var remaining: ArraySlice<UInt8>

	public init(input: [UInt8]) {
		remaining = input[...]
	}

	public init(input: ArraySlice<UInt8>) {
		remaining = input
	}

	public var unconsumed: ArraySlice<UInt8> { remaining }

	private mutating func take(_ len: Int) throws -> ArraySlice<UInt8> {
		guard remaining.count >= len else { throw PodError.truncated }
		let slice = remaining.prefix(len)
		remaining = remaining.dropFirst(len)
		return slice
	}

	public mutating func u8() throws -> UInt8 {
		try take(1).first!
	}

	public mutating func u16() throws -> UInt16 {
		let b = try take(2)
		return UInt16(b[b.startIndex]) | (UInt16(b[b.startIndex + 1]) << 8)
	}

	public mutating func u32() throws -> UInt32 {
		let b = try take(4)
		var v: UInt32 = 0
		for (i, byte) in b.enumerated() {
			v |= UInt32(byte) << (8 * i)
		}
		return v
	}

	public mutating func u64() throws -> UInt64 {
		let b = try take(8)
		var v: UInt64 = 0
		for (i, byte) in b.enumerated() {
			v |= UInt64(byte) << (8 * i)
		}
		return v
	}

	public mutating func i8() throws -> Int8 { Int8(bitPattern: try u8()) }
	public mutating func i16() throws -> Int16 { Int16(bitPattern: try u16()) }
	public mutating func i32() throws -> Int32 { Int32(bitPattern: try u32()) }
	public mutating func i64() throws -> Int64 { Int64(bitPattern: try u64()) }

	public mutating func bool() throws -> Bool {
		switch try u8() {
		case 0: return false
		case 1: return true
		case let other: throw PodError.invalidTag(UInt16(other))
		}
	}

	public mutating func bytes() throws -> ArraySlice<UInt8> {
		let len = Int(try u32())
		return try take(len)
	}

	public mutating func string() throws -> String {
		let slice = try bytes()
		guard let s = String(bytes: slice, encoding: .utf8) else {
			throw PodError.invalidUtf8
		}
		return s
	}

	public mutating func vecLen() throws -> Int {
		Int(try u32())
	}

	public mutating func optionTag() throws -> Bool {
		switch try u8() {
		case 0: return false
		case 1: return true
		case let other: throw PodError.invalidTag(UInt16(other))
		}
	}

	public mutating func resultTag() throws -> Bool {
		switch try u8() {
		case 0: return true
		case 1: return false
		case let other: throw PodError.invalidTag(UInt16(other))
		}
	}

	public mutating func structHeader() throws -> (UInt8, UInt8) {
		(try u8(), try u8())
	}

	public mutating func variant() throws -> UInt16 {
		try u16()
	}

	public func finish() throws {
		guard remaining.isEmpty else { throw PodError.trailing }
	}
}
