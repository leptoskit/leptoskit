package wire
import ("testing"; "encoding/json")
func TestLivePreservesRequiredNull(t *testing.T) {
 args := struct { Note *string `json:"note"`; Missing *string `json:"missing,omitempty"`; Items []string `json:"items"` }{Items: []string{}}
 encoded, err := livePresent(args)
 if err != nil { t.Fatal(err) }
 var actual map[string]json.RawMessage
 if err := json.Unmarshal(encoded, &actual); err != nil { t.Fatal(err) }
 if string(actual["note"]) != "null" || string(actual["items"]) != "[]" { t.Fatalf("lost explicit values: %s",encoded) }
 if _, ok := actual["missing"]; ok { t.Fatalf("sent absent value: %s",encoded) }
}
