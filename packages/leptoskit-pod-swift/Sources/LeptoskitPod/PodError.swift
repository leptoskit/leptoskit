/// Errors from encoding or decoding LK Pod bytes.
public enum PodError: Error, Equatable, Sendable {
	case truncated
	case trailing
	case tooLarge
	case invalidUtf8
	case invalidTag(UInt16)
	case invalidEnvelope
	case unknownVersion(UInt16)
}

extension PodError: CustomDebugStringConvertible {
	public var debugDescription: String {
		switch self {
		case .truncated: "Truncated"
		case .trailing: "Trailing"
		case .tooLarge: "TooLarge"
		case .invalidUtf8: "InvalidUtf8"
		case .invalidTag(let tag): "InvalidTag(\(tag))"
		case .invalidEnvelope: "InvalidEnvelope"
		case .unknownVersion(let v): "UnknownVersion(\(v))"
		}
	}
}

public typealias PodResult<T> = Result<T, PodError>
