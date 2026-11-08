package pod

import (
	"encoding/binary"
	"unicode/utf8"
)

// Reader reads little-endian Pod fields from a buffer.
type Reader struct {
	remaining []byte
}

// NewReader starts at the beginning of input.
func NewReader(input []byte) *Reader {
	return &Reader{remaining: input}
}

// Remaining returns unconsumed bytes.
func (r *Reader) Remaining() []byte {
	return r.remaining
}

func (r *Reader) take(n int) ([]byte, error) {
	if len(r.remaining) < n {
		return nil, errTruncated()
	}
	out := r.remaining[:n]
	r.remaining = r.remaining[n:]
	return out, nil
}

// U8 reads a u8.
func (r *Reader) U8() (uint8, error) {
	b, err := r.take(1)
	if err != nil {
		return 0, err
	}
	return b[0], nil
}

// U16 reads little-endian u16.
func (r *Reader) U16() (uint16, error) {
	b, err := r.take(2)
	if err != nil {
		return 0, err
	}
	return binary.LittleEndian.Uint16(b), nil
}

// U32 reads little-endian u32.
func (r *Reader) U32() (uint32, error) {
	b, err := r.take(4)
	if err != nil {
		return 0, err
	}
	return binary.LittleEndian.Uint32(b), nil
}

// U64 reads little-endian u64.
func (r *Reader) U64() (uint64, error) {
	b, err := r.take(8)
	if err != nil {
		return 0, err
	}
	return binary.LittleEndian.Uint64(b), nil
}

// I8 reads i8.
func (r *Reader) I8() (int8, error) {
	v, err := r.U8()
	return int8(v), err
}

// I16 reads i16.
func (r *Reader) I16() (int16, error) {
	v, err := r.U16()
	return int16(v), err
}

// I32 reads i32.
func (r *Reader) I32() (int32, error) {
	v, err := r.U32()
	return int32(v), err
}

// I64 reads i64.
func (r *Reader) I64() (int64, error) {
	v, err := r.U64()
	return int64(v), err
}

// Bool reads bool (0/1 only).
func (r *Reader) Bool() (bool, error) {
	tag, err := r.U8()
	if err != nil {
		return false, err
	}
	switch tag {
	case 0:
		return false, nil
	case 1:
		return true, nil
	default:
		return false, errInvalidTag(uint16(tag))
	}
}

// Bytes reads a length-prefixed slice.
func (r *Reader) Bytes() ([]byte, error) {
	n, err := r.U32()
	if err != nil {
		return nil, err
	}
	return r.take(int(n))
}

// String reads a UTF-8 string.
func (r *Reader) String() (string, error) {
	raw, err := r.Bytes()
	if err != nil {
		return "", err
	}
	if !utf8.Valid(raw) {
		return "", errInvalidUtf8()
	}
	return string(raw), nil
}

// VecLen reads vector element count.
func (r *Reader) VecLen() (int, error) {
	n, err := r.U32()
	return int(n), err
}

// OptionTag reads option tag.
func (r *Reader) OptionTag() (bool, error) {
	tag, err := r.U8()
	if err != nil {
		return false, err
	}
	switch tag {
	case 0:
		return false, nil
	case 1:
		return true, nil
	default:
		return false, errInvalidTag(uint16(tag))
	}
}

// ResultTag reads result tag (true = Ok payload).
func (r *Reader) ResultTag() (bool, error) {
	tag, err := r.U8()
	if err != nil {
		return false, err
	}
	switch tag {
	case 0:
		return true, nil
	case 1:
		return false, nil
	default:
		return false, errInvalidTag(uint16(tag))
	}
}

// StructHeader reads layout version (little-endian u16).
func (r *Reader) StructHeader() (uint16, error) {
	return r.U16()
}

// Variant reads enum discriminator.
func (r *Reader) Variant() (uint16, error) {
	return r.U16()
}

// Finish requires all bytes consumed.
func (r *Reader) Finish() error {
	if len(r.remaining) == 0 {
		return nil
	}
	return errTrailing()
}
