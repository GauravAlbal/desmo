package invariantkernel

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

const (
	protocolMajorDefault  = 1
	protocolMinorDefault  = 0
	runtimeVersionDefault = "0.5.1"
)

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
	return "", &TransportError{Message: "ik binary not found; pass Options.Binary or set IK_BINARY"}
}

func parseVersion(ctx context.Context, binary string) (runtime string, major, minor uint64, err error) {
	runtime, major, minor = runtimeVersionDefault, protocolMajorDefault, protocolMinorDefault
	cmd := exec.CommandContext(ctx, binary, "--version")
	out, runErr := cmd.Output()
	if runErr != nil {
		return runtime, major, minor, nil
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
	var data map[string]any
	if err := json.Unmarshal(stdout.Bytes(), &data); err != nil {
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
	var envelope map[string]any
	if err := json.Unmarshal(raw, &envelope); err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = "ik returned non-JSON stdout"
		}
		return nil, &TransportError{Op: asString(request["op"]), Message: msg, Err: err}
	}
	if ok, _ := envelope["ok"].(bool); ok {
		result, _ := envelope["result"].(map[string]any)
		if result == nil {
			return nil, &TransportError{Op: asString(request["op"]), Message: "successful envelope missing object result"}
		}
		return result, nil
	}
	if diag, ok := envelope["diagnostic"].(map[string]any); ok {
		return nil, fromDiagnostic(diag)
	}
	return nil, &TransportError{Op: asString(request["op"]), Message: "refused envelope without diagnostic"}
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
