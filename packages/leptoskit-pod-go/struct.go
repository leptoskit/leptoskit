package pod

// EncodeStruct writes version then fields.
func EncodeStruct(version uint16, encodeFields func(*Writer) error) ([]byte, error) {
	w := NewWriter()
	w.StructHeader(version)
	if err := encodeFields(w); err != nil {
		return nil, err
	}
	return w.Finish(), nil
}

// EncodeStructV0 writes version 0 dense struct header then fields.
func EncodeStructV0(encodeFields func(*Writer) error) ([]byte, error) {
	return EncodeStruct(0, encodeFields)
}

// DecodeStructVersion decodes a dense struct at expectedVersion.
func DecodeStructVersion(r *Reader, expectedVersion uint16, decodeFields func(*Reader) error) error {
	version, err := r.StructHeader()
	if err != nil {
		return err
	}
	if version != expectedVersion {
		return errUnknownVersion(version)
	}
	return decodeFields(r)
}
