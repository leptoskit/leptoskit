package main
import (
 "context"
 "encoding/json"
 "io"
 "net/http"
 "os"
 "strings"
 "wire"
)
type observe struct { requests []map[string]string }
func (o *observe) RoundTrip(r *http.Request) (*http.Response, error) {
 body := ""
 if r.Body != nil { b, err := io.ReadAll(r.Body); if err != nil { panic(err) }; body = string(b) }
 o.requests = append(o.requests, map[string]string{"query": r.URL.RawQuery, "body": body})
 return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(`"ok"`)), Header: http.Header{}}, nil
}
func main() {
 transport := &observe{}
 client := wire.NewClient("http://example.invalid")
 client.HTTPClient = &http.Client{Transport: transport}
 empty := ""
 for _, fallback := range []*string{nil, &empty} {
   _, err := client.Probe(context.Background(), []*string{}, map[string]any{}, nil, false, 0, "", fallback)
   if err != nil { panic(err) }
 }
 escaped := "a +&é"
 _, err := client.Probe(context.Background(), []*string{nil, &escaped, &empty}, map[string]any{"key +&é": nil, "empty": ""}, nil, false, 0, escaped, nil)
 if err != nil { panic(err) }
 if err := json.NewEncoder(os.Stdout).Encode(transport.requests); err != nil { panic(err) }
}
