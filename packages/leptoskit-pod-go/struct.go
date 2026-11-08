package pod

// EncodeStructV0 writes version 0 dense struct header then fields.
func EncodeStructV0(encodeFields func(*Writer) error) ([]byte, error) {
	w := NewWriter()
	w.StructHeader(0, 0)
	if err := encodeFields(w); err != nil {
		return nil, err
	}
	return w.Finish(), nil
}

// DecodeStructVersion decodes a dense struct at expectedVersion.
func DecodeStructVersion(r *Reader, expectedVersion uint8, decodeFields func(*Reader) error) error {
	version, flags, err := r.StructHeader()
	if err != nil {
		return err
	}
	if version != expectedVersion {
		return errUnknownVersion(uint16(version))
	}
	if flags != 0 {
		return errInvalidTag(uint16(flags))
	}
	return decodeFields(r)
}
