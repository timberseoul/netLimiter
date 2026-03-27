package ui

import (
	"fmt"
	"sort"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"netlimiter-ui/service"
	"netlimiter-ui/types"
)

// tickMsg triggers a UI refresh at a fixed interval.
// The tick is purely for rendering — data is fetched independently
// by the StatsService background goroutine.
type tickMsg time.Time

// Model is the bubbletea model for the TUI.
type Model struct {
	statsSvc       *service.StatsService
	flows          []types.ProcessFlow
	err            error
	width          int
	height         int
	sortBy         string // "download", "upload", "name", "pid"
	sortAsc        bool
	quitting       bool
	filterCategory string // "" = all, "user", "system", "service"
	scrollOffset   int    // vertical scroll position (0 = top)
	totalRows      int    // total renderable rows (for scroll bounds)
}

// NewModel creates a new TUI model backed by a StatsService.
func NewModel(statsSvc *service.StatsService) Model {
	// Read initial data so the first frame has content.
	flows, lastErr := statsSvc.Snapshot()
	return Model{
		statsSvc:       statsSvc,
		flows:          flows,
		err:            lastErr,
		sortBy:         "download",
		sortAsc:        false,
		filterCategory: "",
		width:          120,
		height:         30,
	}
}

// Init implements tea.Model.
func (m Model) Init() tea.Cmd {
	return tea.Batch(tickCmd(), tea.WindowSize())
}

// Update implements tea.Model.
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "q", "ctrl+c":
			m.quitting = true
			return m, tea.Quit
		case "s":
			if m.sortBy == "download" {
				m.sortBy = "upload"
			} else {
				m.sortBy = "download"
			}
		case "n":
			m.sortBy = "name"
		case "p":
			m.sortBy = "pid"
		case "r":
			m.sortAsc = !m.sortAsc
		case "1":
			m.filterCategory = ""
			m.scrollOffset = 0
		case "2":
			m.filterCategory = "user"
			m.scrollOffset = 0
		case "3":
			m.filterCategory = "system"
			m.scrollOffset = 0
		case "4":
			m.filterCategory = "service"
			m.scrollOffset = 0
		case "up", "k":
			if m.scrollOffset > 0 {
				m.scrollOffset--
			}
		case "down", "j":
			m.scrollOffset++
		case "pgup":
			m.scrollOffset -= 10
			if m.scrollOffset < 0 {
				m.scrollOffset = 0
			}
		case "pgdown":
			m.scrollOffset += 10
		case "home":
			m.scrollOffset = 0
		case "end":
			m.scrollOffset = m.totalRows
		}

	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height

	case tickMsg:
		// Read the latest cached stats from the background service.
		// This is non-blocking — no IPC happens here.
		flows, lastErr := m.statsSvc.Snapshot()
		m.flows = flows
		m.err = lastErr
		return m, tickCmd()
	}

	return m, nil
}

// tickCmd schedules the next UI refresh.
// This fires at a true fixed 1-second interval regardless of IPC latency.
func tickCmd() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg {
		return tickMsg(t)
	})
}

// sortFlows sorts the flow list based on current sort settings.
func (m *Model) sortFlows() []types.ProcessFlow {
	flows := make([]types.ProcessFlow, len(m.flows))
	copy(flows, m.flows)

	sort.Slice(flows, func(i, j int) bool {
		var less bool
		switch m.sortBy {
		case "download":
			less = flows[i].DownloadSpeed > flows[j].DownloadSpeed
		case "upload":
			less = flows[i].UploadSpeed > flows[j].UploadSpeed
		case "name":
			less = flows[i].Name < flows[j].Name
		case "pid":
			less = flows[i].PID < flows[j].PID
		default:
			less = flows[i].DownloadSpeed > flows[j].DownloadSpeed
		}
		if m.sortAsc {
			return !less
		}
		return less
	})

	return flows
}

// formatSpeed converts bytes/sec to a human-readable string.
func formatSpeed(bytesPerSec float64) string {
	switch {
	case bytesPerSec >= 1024*1024*1024:
		return fmt.Sprintf("%.2f GB/s", bytesPerSec/(1024*1024*1024))
	case bytesPerSec >= 1024*1024:
		return fmt.Sprintf("%.2f MB/s", bytesPerSec/(1024*1024))
	case bytesPerSec >= 1024:
		return fmt.Sprintf("%.2f KB/s", bytesPerSec/1024)
	default:
		return fmt.Sprintf("%.0f  B/s", bytesPerSec)
	}
}

// formatBytes converts a byte count to a human-readable string.
func formatBytes(bytes uint64) string {
	b := float64(bytes)
	switch {
	case b >= 1024*1024*1024:
		return fmt.Sprintf("%.2f GB", b/(1024*1024*1024))
	case b >= 1024*1024:
		return fmt.Sprintf("%.2f MB", b/(1024*1024))
	case b >= 1024:
		return fmt.Sprintf("%.1f KB", b/1024)
	default:
		return fmt.Sprintf("%d B", bytes)
	}
}
