package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"netlimiter-ui/types"
)

var (
	titleStyle = lipgloss.NewStyle().
			Bold(true).
			Foreground(lipgloss.Color("#00FF88")).
			Background(lipgloss.Color("#1a1a2e")).
			Padding(0, 1)

	headerStyle = lipgloss.NewStyle().
			Bold(true).
			Foreground(lipgloss.Color("#61AFEF")).
			BorderBottom(true).
			BorderStyle(lipgloss.NormalBorder()).
			BorderForeground(lipgloss.Color("#3B4048"))

	rowStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#ABB2BF"))

	inactiveRowStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("#5C6370"))

	speedUpStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#E06C75"))

	speedDownStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#98C379"))

	dimSpeedStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#4B5263"))

	pidStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#E5C07B"))

	dimPidStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5C6370"))

	nameStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#C678DD"))

	dimNameStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5C6370"))

	helpStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5C6370")).
			Italic(true)

	errorStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#E06C75")).
			Bold(true)

	sortIndicator = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#00FF88")).
			Bold(true)

	categoryHeaderStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("#61AFEF")).
				PaddingLeft(1)

	activeFilterStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("#00FF88"))

	inactiveFilterStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("#5C6370"))

	statusActiveStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("#98C379"))

	statusInactiveStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("#5C6370"))
)

// categoryOrder defines display order and labels.
var categoryOrder = []struct {
	key   string
	label string
}{
	{"user", "👤 User Processes"},
	{"system", "⚙ System Processes"},
	{"service", "🔧 Services"},
}

// View implements tea.Model.
func (m Model) View() string {
	if m.quitting {
		return "Bye!\n"
	}

	var b strings.Builder

	// Title bar
	title := titleStyle.Render(" ⚡ NetLimiter — Real-time Network Monitor ")
	b.WriteString(title + "\n\n")

	// Error display
	if m.err != nil {
		b.WriteString(errorStyle.Render(fmt.Sprintf("  ⚠  Connection error: %v", m.err)) + "\n")
		b.WriteString(errorStyle.Render("     Make sure the Rust core is running.") + "\n\n")
	}

	// Filter tabs
	filterLine := "  Filter: "
	filters := []struct {
		key   string
		label string
	}{
		{"", "All[1]"},
		{"user", "User[2]"},
		{"system", "System[3]"},
		{"service", "Service[4]"},
	}
	for _, f := range filters {
		if m.filterCategory == f.key {
			filterLine += activeFilterStyle.Render(" "+f.label+" ") + " "
		} else {
			filterLine += inactiveFilterStyle.Render(" "+f.label+" ") + " "
		}
	}
	b.WriteString(filterLine + "\n")

	// Sort indicator
	sortLabel := fmt.Sprintf("  Sort: %s", m.sortBy)
	if m.sortAsc {
		sortLabel += " ▲"
	} else {
		sortLabel += " ▼"
	}
	b.WriteString(sortIndicator.Render(sortLabel) + "\n\n")

	// Group flows by category
	flows := m.sortFlows()
	grouped := make(map[string][]types.ProcessFlow)
	for _, f := range flows {
		cat := f.Category
		if cat == "" {
			cat = "unknown"
		}
		if m.filterCategory != "" && cat != m.filterCategory {
			continue
		}
		grouped[cat] = append(grouped[cat], f)
	}

	// Compute dynamic name column width (min 16, fit longest name)
	nameColWidth := 16
	for _, catFlows := range grouped {
		for _, f := range catFlows {
			if len(f.Name) > nameColWidth {
				nameColWidth = len(f.Name)
			}
		}
	}
	// Cap at a reasonable max to prevent extreme widths
	if nameColWidth > 60 {
		nameColWidth = 60
	}

	// Table header (dynamic name width)
	headerFmt := fmt.Sprintf("  %%-8s  %%-%ds  %%-8s  %%12s  %%12s  %%10s  %%10s", nameColWidth)
	header := fmt.Sprintf(headerFmt,
		"PID", "Process", "Status", "↑ Speed", "↓ Speed", "↑ Total", "↓ Total")
	b.WriteString(headerStyle.Render(header) + "\n")

	hasAny := false
	for _, cat := range categoryOrder {
		if len(grouped[cat.key]) > 0 {
			hasAny = true
			break
		}
	}

	// Build all content rows first, then apply scrolling viewport
	var contentLines []string

	if !hasAny {
		contentLines = append(contentLines, rowStyle.Render("  Waiting for network activity..."))
	} else {
		rowFmtName := fmt.Sprintf("%%-%ds", nameColWidth)
		for _, cat := range categoryOrder {
			catFlows, ok := grouped[cat.key]
			if !ok || len(catFlows) == 0 {
				continue
			}

			// Category section header
			contentLines = append(contentLines, "")
			sectionHeader := fmt.Sprintf("── %s (%d) ──", cat.label, len(catFlows))
			contentLines = append(contentLines, categoryHeaderStyle.Render(sectionHeader))

			for _, f := range catFlows {
				name := f.Name
				if len(name) > nameColWidth {
					name = name[:nameColWidth-3] + "..."
				}

				inactive := f.Status == "inactive"
				var row string
				if inactive {
					pid := dimPidStyle.Render(fmt.Sprintf("%-8d", f.PID))
					nameR := dimNameStyle.Render(fmt.Sprintf(rowFmtName, name))
					st := statusInactiveStyle.Render(fmt.Sprintf("%-8s", "idle"))
					up := dimSpeedStyle.Render(fmt.Sprintf("%12s", "—"))
					down := dimSpeedStyle.Render(fmt.Sprintf("%12s", "—"))
					tUp := dimSpeedStyle.Render(fmt.Sprintf("%10s", formatBytes(f.TotalUpload)))
					tDown := dimSpeedStyle.Render(fmt.Sprintf("%10s", formatBytes(f.TotalDownload)))
					row = fmt.Sprintf("  %s  %s  %s  %s  %s  %s  %s", pid, nameR, st, up, down, tUp, tDown)
				} else {
					pid := pidStyle.Render(fmt.Sprintf("%-8d", f.PID))
					nameR := nameStyle.Render(fmt.Sprintf(rowFmtName, name))
					st := statusActiveStyle.Render(fmt.Sprintf("%-8s", "●"))
					up := speedUpStyle.Render(fmt.Sprintf("%12s", formatSpeed(f.UploadSpeed)))
					down := speedDownStyle.Render(fmt.Sprintf("%12s", formatSpeed(f.DownloadSpeed)))
					tUp := rowStyle.Render(fmt.Sprintf("%10s", formatBytes(f.TotalUpload)))
					tDown := rowStyle.Render(fmt.Sprintf("%10s", formatBytes(f.TotalDownload)))
					row = fmt.Sprintf("  %s  %s  %s  %s  %s  %s  %s", pid, nameR, st, up, down, tUp, tDown)
				}

				contentLines = append(contentLines, row)
			}
		}
	}

	// Apply scrolling viewport
	viewportHeight := m.height - 12 // reserve for title, filter, sort, header, footer, totals
	if viewportHeight < 5 {
		viewportHeight = 5
	}

	totalContentRows := len(contentLines)
	// Clamp scroll offset
	maxScroll := totalContentRows - viewportHeight
	if maxScroll < 0 {
		maxScroll = 0
	}
	scrollOff := m.scrollOffset
	if scrollOff > maxScroll {
		scrollOff = maxScroll
	}
	if scrollOff < 0 {
		scrollOff = 0
	}

	endIdx := scrollOff + viewportHeight
	if endIdx > totalContentRows {
		endIdx = totalContentRows
	}

	for _, line := range contentLines[scrollOff:endIdx] {
		b.WriteString(line + "\n")
	}

	// Scroll indicator
	if totalContentRows > viewportHeight {
		scrollInfo := fmt.Sprintf("  ── ↑↓/j/k scroll | showing %d-%d of %d rows ──",
			scrollOff+1, endIdx, totalContentRows)
		b.WriteString(helpStyle.Render(scrollInfo) + "\n")
	}

	// Footer / help
	b.WriteString("\n")
	help := "  [s] Sort speed  [n] Name  [p] PID  [r] Reverse  [1-4] Filter  [↑↓] Scroll  [q] Quit"
	b.WriteString(helpStyle.Render(help) + "\n")

	// Total (respecting filter)
	var totalUp, totalDown float64
	var totalUpBytes, totalDownBytes uint64
	count := 0
	for _, catFlows := range grouped {
		for _, f := range catFlows {
			totalUp += f.UploadSpeed
			totalDown += f.DownloadSpeed
			totalUpBytes += f.TotalUpload
			totalDownBytes += f.TotalDownload
			count++
		}
	}
	total := fmt.Sprintf("  Speed: ↑ %s  ↓ %s  |  Traffic: ↑ %s  ↓ %s  |  %d processes",
		formatSpeed(totalUp), formatSpeed(totalDown),
		formatBytes(totalUpBytes), formatBytes(totalDownBytes), count)
	b.WriteString("\n" + rowStyle.Render(total) + "\n")

	return b.String()
}
