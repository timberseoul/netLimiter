package types

// ProcessFlow represents per-process network statistics from the Rust core.
type ProcessFlow struct {
	PID           uint32  `json:"pid"`
	Name          string  `json:"name"`
	Category      string  `json:"category"`
	Status        string  `json:"status"` // "active" or "inactive"
	UploadSpeed   float64 `json:"upload_speed"`
	DownloadSpeed float64 `json:"download_speed"`
	TotalUpload   uint64  `json:"total_upload"`
	TotalDownload uint64  `json:"total_download"`
}

// IpcResponse is the JSON message received from Rust.
type IpcResponse struct {
	Type  string        `json:"type"`
	Data  []ProcessFlow `json:"data,omitempty"`
	Error string        `json:"error,omitempty"`
}

// IpcRequest is the JSON message sent to Rust.
type IpcRequest struct {
	Command       string   `json:"command"`
	PID           *uint32  `json:"pid,omitempty"`
	UploadLimit   *float64 `json:"upload_limit,omitempty"`
	DownloadLimit *float64 `json:"download_limit,omitempty"`
}
