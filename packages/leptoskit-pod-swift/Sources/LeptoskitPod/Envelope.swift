/// Transport envelope around an RPC / live Pod body.

public let podMagic: [UInt8] = Array("LK01".utf8)
public let podContentType = "application/vnd.leptoskit.rpc.v1+bin"
public let podCodecVersion: UInt16 = 1

public enum MessageType: UInt16, Equatable, Sendable {
	case rpc = 1
	case live = 2

	static func fromU16(_ value: UInt16) throws -> MessageType {
		switch value {
		case 1: return .rpc
		case 2: return .live
		default: throw PodError.invalidTag(value)
		}
	}
}

public struct Envelope: Equatable, Sendable {
	public var codecVer: UInt16
	public var msgType: MessageType
	public var flags: UInt16
	public var body: [UInt8]

	public init(codecVer: UInt16, msgType: MessageType, flags: UInt16, body: [UInt8]) {
		self.codecVer = codecVer
		self.msgType = msgType
		self.flags = flags
		self.body = body
	}
}

public func wrap(msgType: MessageType, flags: UInt16, body: [UInt8]) throws -> [UInt8] {
	guard body.count <= Int(UInt32.max) else { throw PodError.tooLarge }
	var w = Writer(capacity: 14 + body.count)
	w.raw(podMagic)
	w.u16(podCodecVersion)
	w.u16(msgType.rawValue)
	w.u16(flags)
	w.u32(UInt32(body.count))
	w.raw(body)
	return w.asSlice()
}

public func unwrap(_ bytes: [UInt8]) throws -> Envelope {
	guard bytes.count >= 14 else { throw PodError.truncated }
	guard Array(bytes.prefix(4)) == podMagic else { throw PodError.invalidEnvelope }
	var r = Reader(input: Array(bytes.dropFirst(4)))
	let codecVer = try r.u16()
	guard codecVer == podCodecVersion else { throw PodError.invalidEnvelope }
	let msgType = try MessageType.fromU16(try r.u16())
	let flags = try r.u16()
	let bodyLen = Int(try r.u32())
	let body = Array(r.unconsumed)
	guard body.count == bodyLen else { throw PodError.truncated }
	return Envelope(codecVer: codecVer, msgType: msgType, flags: flags, body: body)
}
