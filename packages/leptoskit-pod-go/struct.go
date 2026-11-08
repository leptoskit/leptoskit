package pod

// EncodeStruct writes disc + version then fields.
func EncodeStruct(disc, version uint16, encodeFields func(*Writer) error) ([]byte, error) {
	w := NewWriter()
	w.StructHeader(disc, version)
	if err := encodeFields(w); err != nil {
		return nil, err
	}
	return w.Finish(), nil
}

// EncodeStructV0 writes disc + version 0 dense struct header then fields.
func EncodeStructV0(disc uint16, encodeFields func(*Writer) error) ([]byte, error) {
	return EncodeStruct(disc, 0, encodeFields)
}

// DecodeStructVersion decodes a dense struct at expectedDisc + expectedVersion.
func DecodeStructVersion(r *Reader, expectedDisc, expectedVersion uint16, decodeFields func(*Reader) error) error {
	disc, version, err := r.StructHeader()
	if err != nil {
		return err
	}
	if disc != expectedDisc {
		return errUnknownDiscriminator(disc)
	}
	if version != expectedVersion {
		return errUnknownVersion(version)
	}
	return decodeFields(r)
}
