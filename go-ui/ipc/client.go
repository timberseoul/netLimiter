package ipc

import (
	"bufio"
	"encoding/json"
	"fmt"
	"net"
	"sync"
	"time"

	"netlimiter-ui/types"

	winio "github.com/Microsoft/go-winio"
)

// Client manages the connection to the Rust named pipe server.
type Client struct {
	conn   net.Conn
	reader *bufio.Reader
	mu     sync.Mutex
}

// NewClient creates and connects a new IPC client.
func NewClient() (*Client, error) {
	timeout := 5 * time.Second
	conn, err := winio.DialPipe(PipeName, &timeout)
	if err != nil {
		return nil, fmt.Errorf("failed to connect to pipe %s: %w", PipeName, err)
	}

	return &Client{
		conn:   conn,
		reader: bufio.NewReaderSize(conn, 65536),
	}, nil
}

// Close closes the connection.
func (c *Client) Close() error {
	if c.conn != nil {
		return c.conn.Close()
	}
	return nil
}

// GetStats requests the latest stats from Rust core.
func (c *Client) GetStats() ([]types.ProcessFlow, error) {
	c.mu.Lock()
	defer c.mu.Unlock()

	req := types.IpcRequest{Command: "get_stats"}
	if err := c.sendRequest(req); err != nil {
		return nil, err
	}

	resp, err := c.readResponse()
	if err != nil {
		return nil, err
	}

	if resp.Type == "error" {
		return nil, fmt.Errorf("server error: %s", resp.Error)
	}

	return resp.Data, nil
}

// Ping checks if the Rust core is responsive.
func (c *Client) Ping() error {
	c.mu.Lock()
	defer c.mu.Unlock()

	req := types.IpcRequest{Command: "ping"}
	if err := c.sendRequest(req); err != nil {
		return err
	}

	resp, err := c.readResponse()
	if err != nil {
		return err
	}

	if resp.Type != "ack" {
		return fmt.Errorf("unexpected response type: %s", resp.Type)
	}

	return nil
}

func (c *Client) sendRequest(req types.IpcRequest) error {
	data, err := json.Marshal(req)
	if err != nil {
		return fmt.Errorf("marshal error: %w", err)
	}

	data = append(data, '\n')

	c.conn.SetWriteDeadline(time.Now().Add(3 * time.Second))
	_, err = c.conn.Write(data)
	if err != nil {
		return fmt.Errorf("write error: %w", err)
	}

	return nil
}

func (c *Client) readResponse() (*types.IpcResponse, error) {
	c.conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	line, err := c.reader.ReadBytes('\n')
	if err != nil {
		return nil, fmt.Errorf("read error: %w", err)
	}

	var resp types.IpcResponse
	if err := json.Unmarshal(line, &resp); err != nil {
		return nil, fmt.Errorf("unmarshal error: %w", err)
	}

	return &resp, nil
}
