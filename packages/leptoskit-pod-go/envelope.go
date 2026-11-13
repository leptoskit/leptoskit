package pod

// Magic is LK01.
var Magic = []byte{'L', 'K', '0', '1'}

// ContentType for HTTP Pod bodies.
const ContentType = "application/vnd.leptoskit.rpc.v1+bin"

// CodecVersion is the current envelope codec version.
const CodecVersion uint16 = 1

// MessageType is the envelope message kind.
type MessageType uint16

const (
	MessageRpc  MessageType = 1
	MessageLive MessageType = 2
)

// Envelope is a parsed transport frame.
type Envelope struct {
	CodecVer uint16
	MsgType  MessageType
	Flags    uint16
	Body     []byte
}

func messageTypeFromU16(value uint16) (MessageType, error) {
	switch value {
	case 1:
		return MessageRpc, nil
	case 2:
		return MessageLive, nil
	default:
		return 0, errInvalidTag(value)
	}
}

// Wrap builds an LK01 envelope around body.
func Wrap(msgType MessageType, flags uint16, body []byte) ([]byte, error) {
	if len(body) > 0xffffffff {
		return nil, errTooLarge()
	}
	w := WithCapacity(14 + len(body))
	w.Raw(Magic)
	w.U16(CodecVersion)
	w.U16(uint16(msgType))
	w.U16(flags)
	w.U32(uint32(len(body)))
	w.Raw(body)
	return w.Finish(), nil
}

// Unwrap parses an LK01 envelope.
func Unwrap(bytes []byte) (*Envelope, error) {
	if len(bytes) < 14 {
		return nil, errTruncated()
	}
	if string(bytes[:4]) != string(Magic) {
		return nil, errInvalidEnvelope()
	}
	r := NewReader(bytes[4:])
	codecVer, err := r.U16()
	if err != nil {
		return nil, err
	}
	if codecVer != CodecVersion {
		return nil, errInvalidEnvelope()
	}
	msgRaw, err := r.U16()
	if err != nil {
		return nil, err
	}
	msgType, err := messageTypeFromU16(msgRaw)
	if err != nil {
		return nil, err
	}
	flags, err := r.U16()
	if err != nil {
		return nil, err
	}
	bodyLen, err := r.U32()
	if err != nil {
		return nil, err
	}
	body := r.Remaining()
	if len(body) != int(bodyLen) {
		return nil, errTruncated()
	}
	return &Envelope{
		CodecVer: codecVer,
		MsgType:  msgType,
		Flags:    flags,
		Body:     body,
	}, nil
}
