package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
	"testing"
	"time"

	"github.com/1password/onepassword-sdk-go"
)

type stubResolver struct {
	fn func(ctx context.Context, refs []string) (onepassword.ResolveAllResponse, error)
}

func (s stubResolver) ResolveAll(ctx context.Context, refs []string) (onepassword.ResolveAllResponse, error) {
	return s.fn(ctx, refs)
}

func okFactory(stub resolver) clientFactory {
	return func(ctx context.Context, token string) (resolver, error) {
		return stub, nil
	}
}

type bridge struct {
	in   io.WriteCloser
	out  *bufio.Scanner
	done chan struct{}
}

func startBridge(t *testing.T, factory clientFactory) *bridge {
	t.Helper()
	inR, inW := io.Pipe()
	outR, outW := io.Pipe()
	srv := newServer(factory, outW)
	done := make(chan struct{})
	go func() {
		srv.run(context.Background(), inR)
		outW.Close()
		close(done)
	}()
	t.Cleanup(func() {
		inW.Close()
		waitDone(t, done)
	})
	scanner := bufio.NewScanner(outR)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	return &bridge{in: inW, out: scanner, done: done}
}

func waitDone(t *testing.T, done chan struct{}) {
	t.Helper()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("bridge did not shut down after stdin close")
	}
}

func (b *bridge) sendLine(t *testing.T, line string) {
	t.Helper()
	if _, err := io.WriteString(b.in, line+"\n"); err != nil {
		t.Fatalf("write request: %v", err)
	}
}

// rawRequest mirrors the wire format independently of the production
// types so these tests prove the protocol, not the serializer.
type rawRequest struct {
	Type     string   `json:"type"`
	ID       uint64   `json:"id"`
	Protocol int      `json:"protocol,omitempty"`
	Token    string   `json:"token,omitempty"`
	Refs     []string `json:"refs,omitempty"`
}

func (b *bridge) send(t *testing.T, req rawRequest) {
	t.Helper()
	raw, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal request: %v", err)
	}
	b.sendLine(t, string(raw))
}

type response struct {
	Type          responseType             `json:"type"`
	ID            uint64                   `json:"id"`
	Protocol      int                      `json:"protocol"`
	BridgeVersion string                   `json:"bridge_version"`
	Code          errorCode                `json:"code"`
	Message       string                   `json:"message"`
	Results       map[string]resolveResult `json:"results"`
}

func (b *bridge) recv(t *testing.T) response {
	t.Helper()
	if !b.out.Scan() {
		t.Fatalf("no response: %v", b.out.Err())
	}
	var resp response
	if err := json.Unmarshal(b.out.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal response %q: %v", b.out.Text(), err)
	}
	return resp
}

func initBridge(t *testing.T, b *bridge) {
	t.Helper()
	b.send(t, rawRequest{Type: "init", ID: 1, Protocol: protocolVersion, Token: "ops_test"})
	resp := b.recv(t)
	if resp.Type != "init-ok" || resp.ID != 1 || resp.Protocol != protocolVersion {
		t.Fatalf("unexpected init response: %+v", resp)
	}
}

func TestInitOK(t *testing.T) {
	b := startBridge(t, okFactory(stubResolver{}))
	initBridge(t, b)
}

func TestInitClientError(t *testing.T) {
	factory := func(ctx context.Context, token string) (resolver, error) {
		return nil, errors.New("(401) invalid bearer token")
	}
	b := startBridge(t, factory)
	b.send(t, rawRequest{Type: "init", ID: 1, Protocol: protocolVersion, Token: "ops_bad"})
	resp := b.recv(t)
	if resp.Type != "error" || resp.Code != codeOther {
		t.Fatalf("expected error response, got: %+v", resp)
	}
	if !strings.Contains(resp.Message, "invalid bearer token") {
		t.Fatalf("SDK message must pass through, got: %q", resp.Message)
	}
}

func TestInitUnsupportedProtocol(t *testing.T) {
	b := startBridge(t, okFactory(stubResolver{}))
	b.send(t, rawRequest{Type: "init", ID: 1, Protocol: 99, Token: "ops_test"})
	resp := b.recv(t)
	if resp.Type != "error" || resp.Code != codeUnsupportedProtocol {
		t.Fatalf("expected unsupported_protocol error, got: %+v", resp)
	}
}

func TestSecondInitRejected(t *testing.T) {
	b := startBridge(t, okFactory(stubResolver{}))
	initBridge(t, b)
	b.send(t, rawRequest{Type: "init", ID: 2, Protocol: protocolVersion, Token: "ops_test"})
	resp := b.recv(t)
	if resp.Type != "error" || resp.Code != codeBadRequest {
		t.Fatalf("expected bad_request error, got: %+v", resp)
	}
}

func TestResolveBeforeInit(t *testing.T) {
	b := startBridge(t, okFactory(stubResolver{}))
	b.send(t, rawRequest{Type: "resolve", ID: 1, Refs: []string{"op://v/i/f"}})
	resp := b.recv(t)
	if resp.Type != "error" || resp.Code != codeBadRequest {
		t.Fatalf("expected bad_request error, got: %+v", resp)
	}
}

func TestMalformedLine(t *testing.T) {
	b := startBridge(t, okFactory(stubResolver{}))
	b.sendLine(t, "{not json")
	resp := b.recv(t)
	if resp.Type != "error" || resp.ID != 0 || resp.Code != codeBadRequest {
		t.Fatalf("expected bad_request error with id 0, got: %+v", resp)
	}
}

func TestUnknownRequestType(t *testing.T) {
	b := startBridge(t, okFactory(stubResolver{}))
	b.send(t, rawRequest{Type: "shutdown", ID: 7})
	resp := b.recv(t)
	if resp.Type != "error" || resp.ID != 7 || resp.Code != codeBadRequest {
		t.Fatalf("expected bad_request error, got: %+v", resp)
	}
}

func TestResolveMixedResults(t *testing.T) {
	secret := "hunter2"
	stub := stubResolver{fn: func(ctx context.Context, refs []string) (onepassword.ResolveAllResponse, error) {
		return onepassword.ResolveAllResponse{
			IndividualResponses: map[string]onepassword.Response[onepassword.ResolvedReference, onepassword.ResolveReferenceError]{
				"op://v/i/f": {Content: &onepassword.ResolvedReference{Secret: secret}},
				"op://v/missing/f": {Error: &onepassword.ResolveReferenceError{
					Type: onepassword.ResolveReferenceErrorTypeVariantItemNotFound,
				}},
			},
		}, nil
	}}
	b := startBridge(t, okFactory(stub))
	initBridge(t, b)
	b.send(t, rawRequest{Type: "resolve", ID: 2, Refs: []string{"op://v/i/f", "op://v/missing/f"}})
	resp := b.recv(t)
	if resp.Type != "resolve-ok" || resp.ID != 2 {
		t.Fatalf("expected resolve-ok, got: %+v", resp)
	}
	got := resp.Results["op://v/i/f"]
	if got.Secret == nil || *got.Secret != secret || got.Error != nil {
		t.Fatalf("expected secret for op://v/i/f, got: %+v", got)
	}
	missing := resp.Results["op://v/missing/f"]
	if missing.Secret != nil || missing.Error == nil || missing.Error.Code != codeNotFound {
		t.Fatalf("expected not_found for op://v/missing/f, got: %+v", missing)
	}
}

func TestResolveWholeRequestError(t *testing.T) {
	stub := stubResolver{fn: func(ctx context.Context, refs []string) (onepassword.ResolveAllResponse, error) {
		return onepassword.ResolveAllResponse{}, errors.New("connection reset by peer")
	}}
	b := startBridge(t, okFactory(stub))
	initBridge(t, b)
	b.send(t, rawRequest{Type: "resolve", ID: 2, Refs: []string{"op://v/i/f"}})
	resp := b.recv(t)
	if resp.Type != "error" || resp.ID != 2 || resp.Code != codeOther {
		t.Fatalf("expected whole-request error, got: %+v", resp)
	}
	if !strings.Contains(resp.Message, "connection reset by peer") {
		t.Fatalf("SDK message must pass through, got: %q", resp.Message)
	}
}

func TestEOFExitsLoop(t *testing.T) {
	b := startBridge(t, okFactory(stubResolver{}))
	initBridge(t, b)
	b.in.Close()
	waitDone(t, b.done)
}

func TestMapRefErrorCode(t *testing.T) {
	cases := []struct {
		typ  onepassword.ResolveReferenceErrorTypes
		want errorCode
	}{
		{onepassword.ResolveReferenceErrorTypeVariantItemNotFound, codeNotFound},
		{onepassword.ResolveReferenceErrorTypeVariantVaultNotFound, codeNotFound},
		{onepassword.ResolveReferenceErrorTypeVariantFieldNotFound, codeNotFound},
		{onepassword.ResolveReferenceErrorTypeVariantNoMatchingSections, codeNotFound},
		{onepassword.ResolveReferenceErrorTypeVariantSSHKeyMetadataNotFound, codeNotFound},
		{onepassword.ResolveReferenceErrorTypeVariantParsing, codeInvalidReference},
		{onepassword.ResolveReferenceErrorTypeVariantTooManyVaults, codeOther},
		{onepassword.ResolveReferenceErrorTypeVariantTooManyItems, codeOther},
		{onepassword.ResolveReferenceErrorTypeVariantUnableToParsePrivateKey, codeOther},
	}
	for _, c := range cases {
		if got := mapRefErrorCode(c.typ); got != c.want {
			t.Errorf("mapRefErrorCode(%q) = %q, want %q", c.typ, got, c.want)
		}
	}
}

func TestClassifyError(t *testing.T) {
	cases := []struct {
		err  error
		want errorCode
	}{
		{&onepassword.RateLimitExceededError{}, codeRateLimited},
		{fmt.Errorf("wrapped: %w", &onepassword.RateLimitExceededError{}), codeRateLimited},
		{errors.New("(401) unauthorized"), codeOther},
		{errors.New("tcp: connection refused"), codeOther},
	}
	for _, c := range cases {
		if got := classifyError(c.err); got != c.want {
			t.Errorf("classifyError(%q) = %q, want %q", c.err, got, c.want)
		}
	}
}
