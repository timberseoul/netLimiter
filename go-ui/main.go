package main

import (
	"fmt"
	"os"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"netlimiter-ui/ipc"
	"netlimiter-ui/service"
	"netlimiter-ui/ui"
)

func main() {
	// Connect to the Rust core via named pipe
	client, err := ipc.NewClient()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to connect to NetLimiter core: %v\n", err)
		fmt.Fprintln(os.Stderr, "Make sure the Rust core (netlimiter-core) is running with admin privileges.")
		os.Exit(1)
	}
	defer client.Close()

	// Ping to verify connection
	if err := client.Ping(); err != nil {
		fmt.Fprintf(os.Stderr, "Core not responding: %v\n", err)
		os.Exit(1)
	}

	// Start the background stats polling service (independent goroutine).
	// This decouples IPC latency from the UI refresh rate.
	statsSvc := service.NewStatsService(client, 1*time.Second)
	statsSvc.Start()
	defer statsSvc.Stop()

	// Start the bubbletea TUI — reads cached stats from StatsService
	model := ui.NewModel(statsSvc)
	p := tea.NewProgram(model, tea.WithAltScreen())

	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Error running TUI: %v\n", err)
		os.Exit(1)
	}
}
