package main

import (
	"fmt"
	"os"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"golang.org/x/sys/windows"

	"netlimiter-ui/core"
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

func isRunningAsAdmin() bool {
	// UAC elevation check: true only when this process is running elevated.
	return windows.GetCurrentProcessToken().IsElevated()
}

func main() {
	if !isRunningAsAdmin() {
		showHint(
			"检测到当前未以管理员身份运行。",
			"请以管理员身份重新运行本程序（右键→以管理员身份运行）。",
		)
		os.Exit(1)
	}

	// ── 1. Launch the Rust core (or detect it's already running) ──
	launcher := core.NewLauncher()
	if err := launcher.Start(); err != nil {
		if isAccessDenied(err) {
			showHint("当前会话无权限启动 NetLimiter 核心进程。",
				"请以管理员身份运行本程序（右键→以管理员身份运行）。")
			os.Exit(1)
		}
		showHint(
			fmt.Sprintf("无法启动核心进程：%v", err),
			"请确认 netlimiter-core.exe 与本程序在同一目录，并以管理员身份运行。",
		)
		os.Exit(1)
	}
	defer launcher.Stop() // UI 退出时自动关闭核心进程

	if pid := launcher.CorePID(); pid > 0 {
		fmt.Printf("核心进程已启动 (PID: %d)，正在连接...\n", pid)
	}

	// ── 2. Connect to the Rust core via named pipe ──
	client, err := ipc.NewClient()
	if err != nil {
		if isAccessDenied(err) {
			showHint("当前会话无权限访问 NetLimiter 核心进程。", "请使用管理员权限重新运行。")
			os.Exit(1)
		}
		showHint(
			fmt.Sprintf("无法连接 NetLimiter 核心进程：%v", err),
			"核心进程可能启动失败，请以管理员身份重新运行。",
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
			"请以管理员身份重新运行。",
		)
		os.Exit(1)
	}

	// ── 3. Start polling service & TUI ──
	statsSvc := service.NewStatsService(client, 1*time.Second)
	statsSvc.Start()
	defer statsSvc.Stop()

	model := ui.NewModel(statsSvc)
	p := tea.NewProgram(model, tea.WithAltScreen())

	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Error running TUI: %v\n", err)
		os.Exit(1)
	}
}
