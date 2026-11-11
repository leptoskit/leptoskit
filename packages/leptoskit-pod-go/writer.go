package pod

import (
	"encoding/binary"
)

// Writer appends little-endian Pod fields.
type Writer struct {
	bytes []byte
}

// NewWriter returns an empty writer.
func NewWriter() *Writer {
	return &Writer{}
}

// WithCapacity reserves space for a known encoded size.
func WithCapacity(capacity int) *Writer {
	return &Writer{bytes: make([]byte, 0, capacity)}
}

// Len is the current buffer length.
func (w *Writer) Len() int {
	return len(w.bytes)
}

// IsEmpty reports whether the buffer is empty.
func (w *Writer) IsEmpty() bool {
	return len(w.bytes) == 0
}

// Raw appends bytes with no length prefix.
func (w *Writer) Raw(value []byte) {
	w.bytes = append(w.bytes, value...)
}

// U8 appends a u8.
func (w *Writer) U8(value uint8) {
	w.bytes = append(w.bytes, value)
}

// U16 appends little-endian u16.
func (w *Writer) U16(value uint16) {
	var buf [2]byte
	binary.LittleEndian.PutUint16(buf[:], value)
	w.Raw(buf[:])
}

// U32 appends little-endian u32.
func (w *Writer) U32(value uint32) {
	var buf [4]byte
	binary.LittleEndian.PutUint32(buf[:], value)
	w.Raw(buf[:])
}

// U64 appends little-endian u64.
func (w *Writer) U64(value uint64) {
	var buf [8]byte
	binary.LittleEndian.PutUint64(buf[:], value)
	w.Raw(buf[:])
}

// I8 appends i8 (bit pattern as u8).
func (w *Writer) I8(value int8) {
	w.U8(uint8(value))
}

// I16 appends i16.
func (w *Writer) I16(value int16) {
	w.U16(uint16(value))
}

// I32 appends i32.
func (w *Writer) I32(value int32) {
	w.U32(uint32(value))
}

// I64 appends i64.
func (w *Writer) I64(value int64) {
	w.U64(uint64(value))
}

// Bool appends bool as 0/1.
func (w *Writer) Bool(value bool) {
	if value {
		w.U8(1)
	} else {
		w.U8(0)
	}
}

// Bytes appends u32 length then bytes.
func (w *Writer) Bytes(value []byte) error {
	if len(value) > 0xffffffff {
		return errTooLarge()
	}
	w.U32(uint32(len(value)))
	w.Raw(value)
	return nil
}

// String appends a UTF-8 string.
func (w *Writer) String(value string) error {
	return w.Bytes([]byte(value))
}

// VecLen appends a vector element count.
func (w *Writer) VecLen(len int) error {
	if len > 0xffffffff {
		return errTooLarge()
	}
	w.U32(uint32(len))
	return nil
}

// OptionTag writes 0 = none, 1 = some.
func (w *Writer) OptionTag(some bool) {
	if some {
		w.U8(1)
	} else {
		w.U8(0)
	}
}

// ResultTag writes 0 = ok, 1 = err.
func (w *Writer) ResultTag(ok bool) {
	if ok {
		w.U8(0)
	} else {
		w.U8(1)
	}
}

// StructHeader writes layout version (little-endian u16).
func (w *Writer) StructHeader(version uint16) {
	w.U16(version)
}

// Variant writes enum discriminator u16.
func (w *Writer) Variant(disc uint16) {
	w.U16(disc)
}

// Finish returns the encoded buffer.
func (w *Writer) Finish() []byte {
	out := make([]byte, len(w.bytes))
	copy(out, w.bytes)
	return out
}
