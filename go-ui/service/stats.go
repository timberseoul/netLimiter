package service

import (
	"log"
	"netlimiter-ui/ipc"
	"netlimiter-ui/types"
	"sync"
	"time"
)

// StatsService runs an independent goroutine that polls the Rust core
// at a fixed interval using time.Ticker (true wall-clock cadence, no drift).
// The UI reads cached results via Snapshot() — zero coupling between
// IPC latency and UI refresh rate.
type StatsService struct {
	client   *ipc.Client
	mu       sync.RWMutex
	stats    []types.ProcessFlow
	lastErr  error
	interval time.Duration
	stopCh   chan struct{}
}

// NewStatsService creates a new stats polling service.
func NewStatsService(client *ipc.Client, interval time.Duration) *StatsService {
	return &StatsService{
		client:   client,
		interval: interval,
		stopCh:   make(chan struct{}),
	}
}

// Start begins the background polling goroutine.
// The goroutine fetches stats immediately, then every `interval`.
func (s *StatsService) Start() {
	// Do an initial fetch so the UI has data right away.
	s.poll()

	go func() {
		ticker := time.NewTicker(s.interval)
		defer ticker.Stop()

		for {
			select {
			case <-s.stopCh:
				return
			case <-ticker.C:
				s.poll()
			}
		}
	}()
}

// poll fetches stats from the Rust core and stores the result.
func (s *StatsService) poll() {
	stats, err := s.client.GetStats()
	s.mu.Lock()
	if err != nil {
		log.Printf("StatsService: poll error: %v", err)
		s.lastErr = err
		// Keep stale stats so the UI still shows the last good data.
	} else {
		s.stats = stats
		s.lastErr = nil
	}
	s.mu.Unlock()
}

// Snapshot returns a copy of the latest stats and the last error (if any).
// This is called by the UI on every tick — it is non-blocking and O(n).
func (s *StatsService) Snapshot() ([]types.ProcessFlow, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	result := make([]types.ProcessFlow, len(s.stats))
	copy(result, s.stats)
	return result, s.lastErr
}

// Stop stops the polling goroutine.
func (s *StatsService) Stop() {
	close(s.stopCh)
}
