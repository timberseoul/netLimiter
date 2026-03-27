package types

import (
	"encoding/json"
	"testing"
)

func TestIpcResponseUnmarshal(t *testing.T) {
	data := []byte(`{"type":"stats","data":[{"pid":1001,"name":"chrome.exe","category":"user","status":"active","upload_speed":10.5,"download_speed":20.5,"total_upload":100,"total_download":200}]}`)

	var resp IpcResponse
	if err := json.Unmarshal(data, &resp); err != nil {
		t.Fatalf("json.Unmarshal failed: %v", err)
	}

	if resp.Type != "stats" {
		t.Fatalf("resp.Type = %q, want %q", resp.Type, "stats")
	}
	if len(resp.Data) != 1 {
		t.Fatalf("len(resp.Data) = %d, want 1", len(resp.Data))
	}
	if resp.Data[0].Name != "chrome.exe" || resp.Data[0].PID != 1001 {
		t.Fatalf("unexpected process payload: %+v", resp.Data[0])
	}
}

func TestIpcRequestMarshal(t *testing.T) {
	pid := uint32(2156)
	upload := 1024.0
	download := 2048.0
	req := IpcRequest{
		Command:       "set_limit",
		PID:           &pid,
		UploadLimit:   &upload,
		DownloadLimit: &download,
	}

	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("json.Marshal failed: %v", err)
	}

	jsonText := string(data)
	for _, want := range []string{"\"command\":\"set_limit\"", "\"pid\":2156", "\"upload_limit\":1024", "\"download_limit\":2048"} {
		if !contains(jsonText, want) {
			t.Fatalf("json %q does not contain %q", jsonText, want)
		}
	}
}

func contains(s, sub string) bool {
	return len(sub) == 0 || (len(s) >= len(sub) && func() bool {
		for i := 0; i <= len(s)-len(sub); i++ {
			if s[i:i+len(sub)] == sub {
				return true
			}
		}
		return false
	}())
}