package ui

import (
	"testing"

	"netlimiter-ui/types"
)

func TestFormatSpeed(t *testing.T) {
	cases := []struct {
		name string
		in   float64
		want string
	}{
		{"bytes", 999, "999  B/s"},
		{"kb", 2048, "2.00 KB/s"},
		{"mb", 3 * 1024 * 1024, "3.00 MB/s"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := formatSpeed(tc.in); got != tc.want {
				t.Fatalf("formatSpeed(%v) = %q, want %q", tc.in, got, tc.want)
			}
		})
	}
}

func TestFormatBytes(t *testing.T) {
	cases := []struct {
		name string
		in   uint64
		want string
	}{
		{"bytes", 512, "512 B"},
		{"kb", 1536, "1.5 KB"},
		{"mb", 2 * 1024 * 1024, "2.00 MB"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := formatBytes(tc.in); got != tc.want {
				t.Fatalf("formatBytes(%v) = %q, want %q", tc.in, got, tc.want)
			}
		})
	}
}

func TestSortFlowsByDownload(t *testing.T) {
	m := Model{
		sortBy:  "download",
		sortAsc: false,
		flows: []types.ProcessFlow{
			{PID: 1, Name: "a", DownloadSpeed: 10},
			{PID: 2, Name: "b", DownloadSpeed: 30},
			{PID: 3, Name: "c", DownloadSpeed: 20},
		},
	}

	got := m.sortFlows()
	if len(got) != 3 {
		t.Fatalf("got %d flows, want 3", len(got))
	}
	if got[0].PID != 2 || got[1].PID != 3 || got[2].PID != 1 {
		t.Fatalf("unexpected sort order: got PIDs [%d, %d, %d]", got[0].PID, got[1].PID, got[2].PID)
	}
}
