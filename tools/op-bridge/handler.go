package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"sync"
	"time"

	"github.com/1password/onepassword-sdk-go"
)

const shutdownGrace = 2 * time.Second

// resolver is the narrow slice of onepassword the bridge needs.
//
//	Tests can just substitute a stub.
type resolver interface {
	ResolveAll(ctx context.Context, secretReferences []string) (onepassword.ResolveAllResponse, error)
}

type clientFactory func(ctx context.Context, token string) (resolver, error)

type server struct {
	newClient clientFactory
	client    resolver

	outMu sync.Mutex
	out   *json.Encoder

	active sync.WaitGroup
}

func newServer(factory clientFactory, out io.Writer) *server {
	return &server{newClient: factory, out: json.NewEncoder(out)}
}

// run processes requests until EOF (or a read error) on `in`, then
// cancels active work and waits briefly for it to settle. locket
// has the other end of the pipe, so EOF is a shutdown.
func (s *server) run(ctx context.Context, in io.Reader) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	reader := bufio.NewReaderSize(in, 64*1024)
	for {
		line, err := reader.ReadBytes('\n')
		if trimmed := bytes.TrimSpace(line); len(trimmed) > 0 {
			s.handle(ctx, trimmed)
		}
		if err != nil {
			if !errors.Is(err, io.EOF) {
				log.Printf("op-bridge: stdin read error: %v", err)
			}
			break
		}
	}

	cancel()
	settled := make(chan struct{})
	go func() {
		s.active.Wait()
		close(settled)
	}()
	select {
	case <-settled:
	case <-time.After(shutdownGrace):
	}
}

func (s *server) handle(ctx context.Context, line []byte) {
	var env envelope
	if err := json.Unmarshal(line, &env); err != nil {
		s.sendError(0, codeBadRequest, "malformed request: "+err.Error())
		return
	}
	switch env.Type {
	case reqInit:
		var req initRequest
		if err := json.Unmarshal(line, &req); err != nil {
			s.sendError(env.ID, codeBadRequest, "malformed init request: "+err.Error())
			return
		}
		s.handleInit(ctx, env.ID, req)
	case reqResolve:
		var req resolveRequest
		if err := json.Unmarshal(line, &req); err != nil {
			s.sendError(env.ID, codeBadRequest, "malformed resolve request: "+err.Error())
			return
		}
		s.handleResolve(ctx, env.ID, req)
	default:
		s.sendError(env.ID, codeBadRequest, fmt.Sprintf("unknown request type %q", env.Type))
	}
}

func (s *server) handleInit(ctx context.Context, id uint64, req initRequest) {
	if s.client != nil {
		s.sendError(id, codeBadRequest, "init already received")
		return
	}
	if req.Protocol != protocolVersion {
		s.sendError(id, codeUnsupportedProtocol,
			fmt.Sprintf("bridge speaks protocol %d, got %d", protocolVersion, req.Protocol))
		return
	}
	if req.Token == "" {
		s.sendError(id, codeBadRequest, "init missing token")
		return
	}
	client, err := s.newClient(ctx, req.Token)
	if err != nil {
		s.sendError(id, classifyError(err), "client init failed: "+err.Error())
		return
	}
	s.client = client
	s.send(initOK{Type: respInitOK, ID: id, Protocol: protocolVersion, BridgeVersion: version})
}

func (s *server) handleResolve(ctx context.Context, id uint64, req resolveRequest) {
	if s.client == nil {
		s.sendError(id, codeBadRequest, "resolve before init")
		return
	}
	client := s.client
	s.active.Add(1)
	go func() {
		defer s.active.Done()
		results := make(map[string]resolveResult, len(req.Refs))
		if len(req.Refs) > 0 {
			resp, err := client.ResolveAll(ctx, req.Refs)
			if err != nil {
				s.sendError(id, classifyError(err), "resolve failed: "+err.Error())
				return
			}
			for ref, individual := range resp.IndividualResponses {
				results[ref] = toResult(individual)
			}
		}
		s.send(resolveOK{Type: respResolveOK, ID: id, Results: results})
	}()
}

func toResult(r onepassword.Response[onepassword.ResolvedReference, onepassword.ResolveReferenceError]) resolveResult {
	if r.Error != nil {
		return resolveResult{Error: &bridgeError{
			Code:    mapRefErrorCode(r.Error.Type),
			Message: "reference could not be resolved: " + string(r.Error.Type),
		}}
	}
	if r.Content == nil {
		return resolveResult{Error: &bridgeError{
			Code:    codeInternal,
			Message: "SDK response had neither content nor error",
		}}
	}
	return resolveResult{Secret: &r.Content.Secret}
}

func mapRefErrorCode(t onepassword.ResolveReferenceErrorTypes) errorCode {
	switch t {
	case onepassword.ResolveReferenceErrorTypeVariantItemNotFound,
		onepassword.ResolveReferenceErrorTypeVariantVaultNotFound,
		onepassword.ResolveReferenceErrorTypeVariantFieldNotFound,
		onepassword.ResolveReferenceErrorTypeVariantNoMatchingSections,
		onepassword.ResolveReferenceErrorTypeVariantSSHKeyMetadataNotFound:
		return codeNotFound
	case onepassword.ResolveReferenceErrorTypeVariantParsing:
		return codeInvalidReference
	default:
		return codeOther
	}
}

// Rate limiting is the only failure the SDK exposes as a typed error. everything else
// is reported as `other` with the SDK's message passed through.
func classifyError(err error) errorCode {
	var rateLimited *onepassword.RateLimitExceededError
	if errors.As(err, &rateLimited) {
		return codeRateLimited
	}
	return codeOther
}

func (s *server) send(v any) {
	s.outMu.Lock()
	defer s.outMu.Unlock()
	if err := s.out.Encode(v); err != nil {
		log.Printf("op-bridge: failed to write response: %v", err)
	}
}

func (s *server) sendError(id uint64, code errorCode, message string) {
	s.send(errorResponse{Type: respError, ID: id, Code: code, Message: message})
}
