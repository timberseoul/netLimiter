// Package core manages the lifecycle of the Rust netlimiter-core process.
// The UI automatically launches the core on startup and kills it on exit.
package core

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"

	winio "github.com/Microsoft/go-winio"
)

const (
	coreBinary = "netlimiter-core.exe"
	pipeName   = `\\.\pipe\netlimiter_ipc`
	// Maximum time to wait for the core to create the named pipe.
	pipeReadyTimeout = 8 * time.Second
	// Polling interval while waiting for the pipe.
	pipeCheckInterval = 200 * time.Millisecond
)

// Launcher manages the core process lifecycle.
type Launcher struct {
	cmd     *exec.Cmd
	exePath string
	// true if this launcher started the core (vs. it was already running)
	owned bool
}

// NewLauncher creates a Launcher. Call Start() to launch the core.
func NewLauncher() *Launcher {
	return &Launcher{}
}

// Start ensures the Rust core is running.
//
// 1. If the named pipe already exists (core already running), skip launch.
// 2. Otherwise, find netlimiter-core.exe next to this exe and start it.
// 3. Wait until the pipe is available (with timeout).
//
// Returns an error if the core cannot be started or becomes ready.
func (l *Launcher) Start() error {
	// Check if core is already running (pipe exists)
	if pipeExists() {
		l.owned = false
		return nil
	}

	// Locate the core binary next to the UI executable
	selfExe, err := os.Executable()
	if err != nil {
		return fmt.Errorf("无法获取自身路径: %w", err)
	}
	selfDir := filepath.Dir(selfExe)
	l.exePath = filepath.Join(selfDir, coreBinary)

	if _, err := os.Stat(l.exePath); os.IsNotExist(err) {
		return fmt.Errorf("找不到核心程序: %s\n请确认 %s 与 %s 在同一目录",
			l.exePath, coreBinary, filepath.Base(selfExe))
	}

	// Start the core process with a hidden console window
	l.cmd = exec.Command(l.exePath)
	l.cmd.Dir = selfDir // WinDivert.dll must be in this directory
	l.cmd.SysProcAttr = &syscall.SysProcAttr{
		CreationFlags: 0x08000000, // CREATE_NO_WINDOW
	}
	// Discard stdout/stderr — core uses its own log file
	l.cmd.Stdout = nil
	l.cmd.Stderr = nil

	if err := l.cmd.Start(); err != nil {
		return fmt.Errorf("启动核心进程失败: %w", err)
	}
	l.owned = true

	// Wait for the pipe to become available
	if err := l.waitForPipe(); err != nil {
		// Core started but pipe never appeared — kill it
		_ = l.Stop()
		return fmt.Errorf("核心进程已启动但命名管道未就绪: %w", err)
	}

	return nil
}

// Stop kills the core process if this launcher owns it.
func (l *Launcher) Stop() error {
	if !l.owned || l.cmd == nil || l.cmd.Process == nil {
		return nil
	}
	// Kill the core process tree
	err := l.cmd.Process.Kill()
	if err != nil {
		return err
	}
	// Wait to reap the process (avoid zombies)
	_ = l.cmd.Wait()
	return nil
}

// CorePID returns the PID of the launched core process, or 0 if not owned.
func (l *Launcher) CorePID() int {
	if l.owned && l.cmd != nil && l.cmd.Process != nil {
		return l.cmd.Process.Pid
	}
	return 0
}

// waitForPipe polls for the named pipe to appear within the timeout.
func (l *Launcher) waitForPipe() error {
	deadline := time.Now().Add(pipeReadyTimeout)
	for time.Now().Before(deadline) {
		// Also check that the core process hasn't exited early
		if l.cmd.ProcessState != nil {
			return fmt.Errorf("核心进程已意外退出 (exit code %d)",
				l.cmd.ProcessState.ExitCode())
		}
		if pipeExists() {
			return nil
		}
		time.Sleep(pipeCheckInterval)
	}
	return fmt.Errorf("等待命名管道超时 (%v)", pipeReadyTimeout)
}

// pipeExists checks if the named pipe is connectable right now.
func pipeExists() bool {
	timeout := 100 * time.Millisecond
	conn, err := winio.DialPipe(pipeName, &timeout)
	if err != nil {
		return false
	}
	conn.Close()
	return true
}
