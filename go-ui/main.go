package main

import (
	"fmt"
	"os"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"netlimiter-ui/ipc"
	"netlimiter-ui/service"
	"netlimiter-ui/ui"
)

type adminHintModel struct {
	message string
	hint    string
}

func (m adminHintModel) Init() tea.Cmd { return nil }

func (m adminHintModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	if key, ok := msg.(tea.KeyMsg); ok {
		switch key.String() {
		case "q", "ctrl+c":
			return m, tea.Quit
		}
	}
	return m, nil
}

func (m adminHintModel) View() string {
	return fmt.Sprintf(
		"\n  ⚠  %s\n\n  %s\n\n  按 q 退出。\n",
		m.message, m.hint,
	)
}

func showHint(msg, hint string) {
	p := tea.NewProgram(adminHintModel{message: msg, hint: hint}, tea.WithAltScreen())
	_, _ = p.Run()
}

func isAccessDenied(err error) bool {
	if err == nil {
		return false
	}
	s := strings.ToLower(err.Error())
	return strings.Contains(s, "access is denied") || strings.Contains(s, "拒绝访问")
}

func main() {
	// Connect to the Rust core via named pipe
	client, err := ipc.NewClient()
	if err != nil {
		if isAccessDenied(err) {
			showHint("当前会话无权限访问 NetLimiter 核心进程。", "请使用管理员权限重新运行。")
			os.Exit(1)
		}
		showHint(
			fmt.Sprintf("无法连接 NetLimiter 核心进程：%v", err),
			"请先启动 netlimiter-core（推荐执行 .\\scripts\\run.ps1）。",
		)
		os.Exit(1)
	}
	defer client.Close()

	// Ping to verify connection
	if err := client.Ping(); err != nil {
		if isAccessDenied(err) {
			showHint("当前会话无权限访问 NetLimiter 核心进程。", "请使用管理员权限重新运行。")
			os.Exit(1)
		}
		showHint(
			fmt.Sprintf("核心进程无响应：%v", err),
			"请确认 netlimiter-core 正在运行，并重试 .\\scripts\\run.ps1。",
		)
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
