package invariantkernel

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

const (
	protocolMajorDefault  = 1
	protocolMinorDefault  = 0
	runtimeVersionDefault = "0.6.0"
	maxSessionLineBytes   = 4 * 1024 * 1024
)

func decodeJSONMap(raw []byte) (map[string]any, error) {
	var out map[string]any
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	if err := dec.Decode(&out); err != nil {
		return nil, err
	}
	return out, nil
}

func locateBinary(explicit string) (string, error) {
	if explicit != "" {
		return explicit, nil
	}
	if env := os.Getenv("IK_BINARY"); env != "" {
		return env, nil
	}
	if env := os.Getenv("IK_BIN"); env != "" {
		return env, nil
	}
	if found, err := exec.LookPath("ik"); err == nil {
		return found, nil
	}
	return "", &TransportError{Message: "ik binary not found; pass Options.Binary or set IK_BINARY", Unsent: true}
}

func parseVersion(ctx context.Context, binary string) (runtime string, major, minor uint64, err error) {
	runtime, major, minor = runtimeVersionDefault, protocolMajorDefault, protocolMinorDefault
	cmd := exec.CommandContext(ctx, binary, "--version")
	out, runErr := cmd.Output()
	if runErr != nil {
		return "", 0, 0, &TransportError{Op: "version", Message: runErr.Error(), Err: runErr, Unsent: neverStarted(runErr)}
	}
	parts := strings.Fields(string(out))
	if len(parts) >= 2 {
		runtime = parts[1]
	}
	if len(parts) >= 4 {
		left, right, ok := strings.Cut(parts[3], ".")
		if ok {
			fmt.Sscanf(left, "%d", &major)
			fmt.Sscanf(right, "%d", &minor)
		}
	}
	return runtime, major, minor, nil
}

func runIdentity(ctx context.Context, binary, machine string) (map[string]any, error) {
	cmd := exec.CommandContext(ctx, binary, "identity", machine)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = strings.TrimSpace(stdout.String())
		}
		if msg == "" {
			msg = err.Error()
		}
		return nil, &TransportError{Op: "identity", Message: msg, Err: err}
	}
	data, err := decodeJSONMap(stdout.Bytes())
	if err != nil {
		return nil, &TransportError{Op: "identity", Message: "ik identity returned non-JSON", Err: err}
	}
	return data, nil
}

func runCheck(ctx context.Context, binary, machine string, prior, corpus, baseline string) (string, bool, error) {
	args := []string{"check", machine}
	if prior != "" {
		args = append(args, "--prior", prior)
	}
	if corpus != "" {
		args = append(args, "--corpus", corpus)
	}
	if baseline != "" {
		args = append(args, "--baseline", baseline)
	}
	cmd := exec.CommandContext(ctx, binary, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	if err != nil {
		if ee, ok := err.(*exec.ExitError); ok && ee.ExitCode() == 1 {
			return stdout.String(), false, nil
		}
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = err.Error()
		}
		return "", false, &TransportError{Op: "check", Message: msg, Err: err}
	}
	return stdout.String(), true, nil
}

func runEvents(ctx context.Context, binary, store string, since uint64, aggregateID string, limit uint64) (*EventFeed, error) {
	args := []string{"events", store, "--since", fmt.Sprintf("%d", since)}
	if aggregateID != "" {
		args = append(args, "--aggregate", aggregateID)
	}
	if limit > 0 {
		args = append(args, "--limit", fmt.Sprintf("%d", limit))
	}
	args = append(args, "--json")
	cmd := exec.CommandContext(ctx, binary, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = strings.TrimSpace(stdout.String())
		}
		if msg == "" {
			msg = err.Error()
		}
		return nil, &TransportError{Op: "events", Message: msg, Err: err, Unsent: neverStarted(err)}
	}
	dec := json.NewDecoder(bytes.NewReader(stdout.Bytes()))
	dec.UseNumber()
	var feed EventFeed
	if err := dec.Decode(&feed); err != nil {
		return nil, &TransportError{Op: "events", Message: "ik events returned invalid JSON", Err: err}
	}
	if feed.Schema != "ik.events.v1" {
		return nil, &TransportError{Op: "events", Message: fmt.Sprintf("ik events returned unexpected schema %q", feed.Schema)}
	}
	return &feed, nil
}

type wireAggregateSummary struct {
	AggregateID     string         `json:"aggregate_id"`
	MachineID       string         `json:"machine_id"`
	MachineVersion  uint64         `json:"machine_version"`
	MachineSpecHash string         `json:"machine_spec_hash"`
	State           any            `json:"state"`
	StateName       string         `json:"state_name"`
	Terminal        bool           `json:"terminal"`
	Generation      uint64         `json:"generation"`
	Sequence        uint64         `json:"sequence"`
	Effect          map[string]any `json:"effect"`
	EffectStatus    string         `json:"effect_status"`
}

type wireAggregateList struct {
	Schema     string                 `json:"schema"`
	Store      string                 `json:"store"`
	NextAfter  string                 `json:"next_after"`
	HasMore    bool                   `json:"has_more"`
	Aggregates []wireAggregateSummary `json:"aggregates"`
}

func runList(ctx context.Context, binary, store string, opts ListOptions) (*AggregateList, error) {
	args := []string{"list", store}
	if opts.MachineID != "" {
		args = append(args, "--machine", opts.MachineID)
	}
	if opts.State != "" {
		args = append(args, "--state", opts.State)
	}
	if opts.Effect != "" {
		args = append(args, "--effect", opts.Effect)
	}
	if opts.Terminal != nil {
		if *opts.Terminal {
			args = append(args, "--terminal", "true")
		} else {
			args = append(args, "--terminal", "false")
		}
	}
	if opts.After != "" {
		args = append(args, "--after", opts.After)
	}
	if opts.Limit > 0 {
		args = append(args, "--limit", fmt.Sprintf("%d", opts.Limit))
	}
	args = append(args, "--json")
	cmd := exec.CommandContext(ctx, binary, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = strings.TrimSpace(stdout.String())
		}
		if msg == "" {
			msg = err.Error()
		}
		return nil, &TransportError{Op: "list", Message: msg, Err: err, Unsent: neverStarted(err)}
	}
	dec := json.NewDecoder(bytes.NewReader(stdout.Bytes()))
	dec.UseNumber()
	var wire wireAggregateList
	if err := dec.Decode(&wire); err != nil {
		return nil, &TransportError{Op: "list", Message: "ik list returned invalid JSON", Err: err}
	}
	if wire.Schema != "ik.list.v1" {
		return nil, &TransportError{Op: "list", Message: fmt.Sprintf("ik list returned unexpected schema %q", wire.Schema)}
	}
	aggs := make([]AggregateSummary, len(wire.Aggregates))
	for i, row := range wire.Aggregates {
		eff := effectFromWire(row.Effect)
		effStatus := row.EffectStatus
		if effStatus == "" {
			effStatus = eff.Status
		}
		aggs[i] = AggregateSummary{
			AggregateID:     row.AggregateID,
			MachineID:       row.MachineID,
			MachineVersion:  row.MachineVersion,
			MachineSpecHash: row.MachineSpecHash,
			State:           row.State,
			StateName:       row.StateName,
			Terminal:        row.Terminal,
			Generation:      row.Generation,
			Sequence:        row.Sequence,
			Effect:          eff,
			EffectStatus:    effStatus,
		}
	}
	return &AggregateList{
		Schema:     wire.Schema,
		Store:      wire.Store,
		NextAfter:  wire.NextAfter,
		HasMore:    wire.HasMore,
		Aggregates: aggs,
	}, nil
}

func invoke(ctx context.Context, binary, store, machine string, request map[string]any) (map[string]any, error) {
	payload, err := json.Marshal(request)
	if err != nil {
		return nil, &TransportError{Op: asString(request["op"]), Message: "encode request", Err: err}
	}
	args := []string{"--store", store}
	if machine != "" {
		args = append(args, "--machine", machine)
	}
	args = append(args, asString(request["op"]))
	cmd := exec.CommandContext(ctx, binary, args...)
	cmd.Stdin = bytes.NewReader(payload)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	runErr := cmd.Run()
	if runErr != nil {
		if _, ok := runErr.(*exec.ExitError); !ok {
			if ctx.Err() != nil {
				return nil, &TransportError{Op: asString(request["op"]), Message: "timed out or canceled", Err: ctx.Err()}
			}
			return nil, &TransportError{Op: asString(request["op"]), Message: runErr.Error(), Err: runErr, Unsent: neverStarted(runErr)}
		}
	}
	raw := bytes.TrimSpace(stdout.Bytes())
	if len(raw) == 0 {
		msg := strings.TrimSpace(stderr.String())
		if ctx.Err() != nil {
			msg = "timed out or canceled"
		}
		if msg == "" {
			msg = "ik returned empty stdout"
		}
		return nil, &TransportError{Op: asString(request["op"]), Message: msg, Err: runErr}
	}
	envelope, err := decodeJSONMap(raw)
	if err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = "ik returned non-JSON stdout"
		}
		return nil, &TransportError{Op: asString(request["op"]), Message: msg, Err: err}
	}
	return unwrapGoEnvelope(asString(request["op"]), envelope)
}

func unwrapGoEnvelope(op string, envelope map[string]any) (map[string]any, error) {
	if ok, _ := envelope["ok"].(bool); ok {
		result, _ := envelope["result"].(map[string]any)
		if result == nil {
			return nil, &TransportError{Op: op, Message: "successful envelope missing object result"}
		}
		return result, nil
	}
	if diag, ok := envelope["diagnostic"].(map[string]any); ok {
		return nil, fromDiagnostic(diag)
	}
	return nil, &TransportError{Op: op, Message: "refused envelope without diagnostic"}
}

type sessionProc struct {
	mu     sync.Mutex
	cmd    *exec.Cmd
	stdin  io.WriteCloser
	lines  chan sessionLine
	stderr bytes.Buffer
	broken bool
}

type sessionLine struct {
	data []byte
	err  error
}

func startSession(binary, store, machine string) (*sessionProc, error) {
	args := []string{"--store", store}
	if machine != "" {
		args = append(args, "--machine", machine)
	}
	args = append(args, "--session")
	cmd := exec.Command(binary, args...)
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, &TransportError{Op: "session", Message: err.Error(), Err: err, Unsent: true}
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, &TransportError{Op: "session", Message: err.Error(), Err: err, Unsent: true}
	}
	s := &sessionProc{
		cmd:   cmd,
		stdin: stdin,
		lines: make(chan sessionLine, 1),
	}
	cmd.Stderr = &s.stderr
	if err := cmd.Start(); err != nil {
		return nil, &TransportError{Op: "session", Message: err.Error(), Err: err, Unsent: neverStarted(err)}
	}
	go func() {
		reader := bufio.NewReaderSize(stdout, 64*1024)
		for {
			line, readErr := reader.ReadBytes('\n')
			if len(line) > maxSessionLineBytes {
				s.lines <- sessionLine{err: errors.New("ik session line exceeded limit")}
				close(s.lines)
				return
			}
			if len(line) > 0 {
				s.lines <- sessionLine{data: bytes.TrimSpace(line)}
			}
			if readErr != nil {
				if !errors.Is(readErr, io.EOF) {
					s.lines <- sessionLine{err: readErr}
				}
				close(s.lines)
				return
			}
		}
	}()
	return s, nil
}

func (s *sessionProc) invoke(ctx context.Context, request map[string]any) (map[string]any, error) {
	op := asString(request["op"])
	payload, err := json.Marshal(request)
	if err != nil {
		return nil, &TransportError{Op: op, Message: "encode request", Err: err, Unsent: true}
	}
	payload = append(payload, '\n')
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.broken {
		return nil, &TransportError{Op: op, Message: "ik session unusable", Unsent: true}
	}
	if _, err := s.stdin.Write(payload); err != nil {
		s.killLocked()
		return nil, &TransportError{Op: op, Message: err.Error(), Err: err}
	}
	select {
	case <-ctx.Done():
		s.killLocked()
		return nil, &TransportError{Op: op, Message: "timed out or canceled", Err: ctx.Err()}
	case line, ok := <-s.lines:
		if !ok {
			s.killLocked()
			msg := strings.TrimSpace(s.stderr.String())
			if msg == "" {
				msg = "ik session closed unexpectedly"
			}
			return nil, &TransportError{Op: op, Message: msg}
		}
		if line.err != nil {
			s.killLocked()
			return nil, &TransportError{Op: op, Message: line.err.Error(), Err: line.err}
		}
		envelope, err := decodeJSONMap(line.data)
		if err != nil {
			s.killLocked()
			return nil, &TransportError{Op: op, Message: "ik session returned non-JSON", Err: err}
		}
		return unwrapGoEnvelope(op, envelope)
	}
}

func (s *sessionProc) killLocked() {
	s.broken = true
	_ = s.stdin.Close()
	if s.cmd.Process != nil {
		_ = s.cmd.Process.Kill()
	}
	_ = s.cmd.Wait()
}

func (s *sessionProc) close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.broken {
		return nil
	}
	s.broken = true
	_ = s.stdin.Close()
	return s.cmd.Wait()
}

// neverStarted reports whether Run failed before the process existed, so the request was
// provably not dispatched.
func neverStarted(err error) bool {
	var execErr *exec.Error
	var pathErr *fs.PathError
	return errors.As(err, &execErr) || (errors.As(err, &pathErr) && pathErr.Op == "fork/exec")
}

func resolvePath(p string) string {
	if p == "" {
		return ""
	}
	abs, err := filepath.Abs(p)
	if err != nil {
		return p
	}
	return abs
}

func defaultTimeout(d time.Duration) time.Duration {
	if d <= 0 {
		return 30 * time.Second
	}
	return d
}
