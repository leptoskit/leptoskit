package pod

import "fmt"

// ErrorCode names match the Rust leptoskit_pod::Error variants.
type ErrorCode string

const (
	ErrTruncated        ErrorCode = "Truncated"
	ErrTrailing         ErrorCode = "Trailing"
	ErrTooLarge         ErrorCode = "TooLarge"
	ErrInvalidUtf8      ErrorCode = "InvalidUtf8"
	ErrInvalidTag       ErrorCode = "InvalidTag"
	ErrInvalidEnvelope      ErrorCode = "InvalidEnvelope"
	ErrUnknownDiscriminator ErrorCode = "UnknownDiscriminator"
	ErrUnknownVersion       ErrorCode = "UnknownVersion"
)

// Error is a codec failure with a stable conceptual class name.
type Error struct {
	Code    ErrorCode
	Tag     uint16
	Disc    uint16
	Version uint16
	msg     string
}

func (e *Error) Error() string {
	if e.msg != "" {
		return e.msg
	}
	return string(e.Code)
}

func errTruncated() *Error {
	return &Error{Code: ErrTruncated, msg: "truncated pod buffer"}
}

func errTrailing() *Error {
	return &Error{Code: ErrTrailing, msg: "trailing bytes after pod value"}
}

func errTooLarge() *Error {
	return &Error{Code: ErrTooLarge, msg: "pod value too large"}
}

func errInvalidUtf8() *Error {
	return &Error{Code: ErrInvalidUtf8, msg: "invalid utf-8 in pod string"}
}

func errInvalidTag(tag uint16) *Error {
	return &Error{Code: ErrInvalidTag, Tag: tag, msg: fmt.Sprintf("invalid pod tag %d", tag)}
}

func errInvalidEnvelope() *Error {
	return &Error{Code: ErrInvalidEnvelope, msg: "invalid pod envelope"}
}

func errUnknownDiscriminator(disc uint16) *Error {
	return &Error{Code: ErrUnknownDiscriminator, Disc: disc, msg: fmt.Sprintf("unknown pod struct discriminator %d", disc)}
}

func errUnknownVersion(v uint16) *Error {
	return &Error{Code: ErrUnknownVersion, Version: v, msg: fmt.Sprintf("unknown pod struct version %d", v)}
}
