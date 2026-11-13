package pod

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

type vector struct {
	Name     string `json:"name"`
	Hex      string `json:"hex"`
	Envelope bool   `json:"envelope"`
}

type malformedEntry struct {
	Name     string `json:"name"`
	Hex      string `json:"hex"`
	Expect   string `json:"expect"`
	Envelope bool   `json:"envelope"`
}

type corpus struct {
	Vectors   []vector         `json:"vectors"`
	Malformed []malformedEntry `json:"malformed"`
}

func vectorsJSONPath(t *testing.T) string {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller")
	}
	return filepath.Join(filepath.Dir(file), "../../crates/leptoskit_pod/conformance/vectors.json")
}

func loadCorpus(t *testing.T) corpus {
	t.Helper()
	raw, err := os.ReadFile(vectorsJSONPath(t))
	if err != nil {
		t.Fatal(err)
	}
	var c corpus
	if err := json.Unmarshal(raw, &c); err != nil {
		t.Fatal(err)
	}
	return c
}

func hexEncode(bytes []byte) string {
	return hex.EncodeToString(bytes)
}

func hexDecode(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func encodeNote(priority uint8, archived bool, title string) ([]byte, error) {
	return EncodeStructV0(func(w *Writer) error {
		w.U8(priority)
		w.Bool(archived)
		return w.String(title)
	})
}

func decodeNote(t *testing.T, r *Reader) {
	t.Helper()
	err := DecodeStructVersion(r, 0, func(inner *Reader) error {
		if _, err := inner.U8(); err != nil {
			return err
		}
		if _, err := inner.Bool(); err != nil {
			return err
		}
		if _, err := inner.String(); err != nil {
			return err
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func encodeVector(t *testing.T, name string) []byte {
	t.Helper()
	switch name {
	case "empty_struct_v0":
		b, err := EncodeStructV0(func(w *Writer) error { return nil })
		if err != nil {
			t.Fatal(err)
		}
		return b
	case "note_hi":
		b, err := encodeNote(7, true, "hi")
		if err != nil {
			t.Fatal(err)
		}
		return b
	case "option_none_u8":
		w := NewWriter()
		w.OptionTag(false)
		return w.Finish()
	case "option_some_u8_9":
		w := NewWriter()
		w.OptionTag(true)
		w.U8(9)
		return w.Finish()
	case "result_ok_u8_3":
		w := NewWriter()
		w.ResultTag(true)
		w.U8(3)
		return w.Finish()
	case "result_err_u8_4":
		w := NewWriter()
		w.ResultTag(false)
		w.U8(4)
		return w.Finish()
	case "vec_u8_empty":
		w := NewWriter()
		if err := w.VecLen(0); err != nil {
			t.Fatal(err)
		}
		return w.Finish()
	case "vec_u8_1_2_3":
		w := NewWriter()
		if err := w.VecLen(3); err != nil {
			t.Fatal(err)
		}
		w.U8(1)
		w.U8(2)
		w.U8(3)
		return w.Finish()
	case "enum_unit_variant_0":
		w := NewWriter()
		w.Variant(0)
		return w.Finish()
	case "enum_struct_variant":
		w := NewWriter()
		w.Variant(1)
		w.StructHeader(0)
		w.U8(7)
		if err := w.String("abc"); err != nil {
			t.Fatal(err)
		}
		return w.Finish()
	case "envelope_rpc_empty":
		b, err := Wrap(MessageRpc, 0, nil)
		if err != nil {
			t.Fatal(err)
		}
		return b
	default:
		t.Fatalf("no encoder for %s", name)
		return nil
	}
}

func decodeVector(t *testing.T, name string, bytes []byte) {
	t.Helper()
	if name == "envelope_rpc_empty" {
		if _, err := Unwrap(bytes); err != nil {
			t.Fatal(err)
		}
		return
	}
	r := NewReader(bytes)
	switch name {
	case "empty_struct_v0":
		version, err := r.StructHeader()
		if err != nil || version != 0 {
			t.Fatal(err)
		}
	case "note_hi":
		decodeNote(t, r)
	case "option_none_u8":
		some, err := r.OptionTag()
		if err != nil || some {
			t.Fatal(err)
		}
	case "option_some_u8_9":
		some, err := r.OptionTag()
		if err != nil || !some {
			t.Fatal(err)
		}
		if _, err := r.U8(); err != nil {
			t.Fatal(err)
		}
	case "result_ok_u8_3":
		ok, err := r.ResultTag()
		if err != nil || !ok {
			t.Fatal(err)
		}
		if _, err := r.U8(); err != nil {
			t.Fatal(err)
		}
	case "result_err_u8_4":
		ok, err := r.ResultTag()
		if err != nil || ok {
			t.Fatal(err)
		}
		if _, err := r.U8(); err != nil {
			t.Fatal(err)
		}
	case "vec_u8_empty":
		n, err := r.VecLen()
		if err != nil || n != 0 {
			t.Fatal(err)
		}
	case "vec_u8_1_2_3":
		n, err := r.VecLen()
		if err != nil || n != 3 {
			t.Fatal(err)
		}
		for i := 0; i < 3; i++ {
			if _, err := r.U8(); err != nil {
				t.Fatal(err)
			}
		}
	case "enum_unit_variant_0":
		d, err := r.Variant()
		if err != nil || d != 0 {
			t.Fatal(err)
		}
	case "enum_struct_variant":
		d, err := r.Variant()
		if err != nil || d != 1 {
			t.Fatal(err)
		}
		err = DecodeStructVersion(r, 0, func(inner *Reader) error {
			if _, err := inner.U8(); err != nil {
				return err
			}
			_, err := inner.String()
			return err
		})
		if err != nil {
			t.Fatal(err)
		}
	default:
		t.Fatalf("no decoder for %s", name)
	}
	if err := r.Finish(); err != nil {
		t.Fatal(err)
	}
}

func TestConformanceVectors(t *testing.T) {
	c := loadCorpus(t)
	for _, v := range c.Vectors {
		v := v
		t.Run(v.Name, func(t *testing.T) {
			want := hexDecode(t, v.Hex)
			got := encodeVector(t, v.Name)
			if hexEncode(got) != v.Hex {
				t.Fatalf("encode %s: got %s want %s", v.Name, hexEncode(got), v.Hex)
			}
			decodeVector(t, v.Name, want)
		})
	}
}

func TestConformanceMalformed(t *testing.T) {
	c := loadCorpus(t)
	for _, m := range c.Malformed {
		m := m
		t.Run(m.Name, func(t *testing.T) {
			bytes := hexDecode(t, m.Hex)
			var err error
			switch {
			case m.Envelope:
				_, err = Unwrap(bytes)
			case strings.Contains(m.Name, "utf8"):
				_, err = NewReader(bytes).String()
			case strings.Contains(m.Name, "bool"):
				_, err = NewReader(bytes).Bool()
			case strings.Contains(m.Name, "option"):
				_, err = NewReader(bytes).OptionTag()
			case strings.Contains(m.Name, "result"):
				_, err = NewReader(bytes).ResultTag()
			case strings.Contains(m.Name, "vec") || strings.Contains(m.Name, "huge_len"):
				_, err = NewReader(bytes).Bytes()
			default:
				_, err = NewReader(bytes).U16()
			}
			podErr, ok := err.(*Error)
			if !ok || podErr == nil {
				t.Fatalf("expected *pod.Error, got %v", err)
			}
			if string(podErr.Code) != m.Expect {
				t.Fatalf("got %s want %s", podErr.Code, m.Expect)
			}
		})
	}
}
