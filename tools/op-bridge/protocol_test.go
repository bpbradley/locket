package main

import (
	"encoding/json"
	"testing"
)

func TestInitRequestDecode(t *testing.T) {
	line := []byte(`{"type":"init","id":1,"protocol":1,"token":"ops_abc"}`)
	var env envelope
	if err := json.Unmarshal(line, &env); err != nil {
		t.Fatalf("unmarshal envelope: %v", err)
	}
	if env.Type != reqInit || env.ID != 1 {
		t.Fatalf("got envelope %+v", env)
	}
	var req initRequest
	if err := json.Unmarshal(line, &req); err != nil {
		t.Fatalf("unmarshal payload: %v", err)
	}
	if req != (initRequest{Protocol: 1, Token: "ops_abc"}) {
		t.Fatalf("got payload %+v", req)
	}
}

func TestResolveRequestDecode(t *testing.T) {
	line := []byte(`{"type":"resolve","id":2,"refs":["op://v/i/f","op://v/i/s/f?ssh-format=openssh"]}`)
	var env envelope
	if err := json.Unmarshal(line, &env); err != nil {
		t.Fatalf("unmarshal envelope: %v", err)
	}
	if env.Type != reqResolve || env.ID != 2 {
		t.Fatalf("got envelope %+v", env)
	}
	var req resolveRequest
	if err := json.Unmarshal(line, &req); err != nil {
		t.Fatalf("unmarshal payload: %v", err)
	}
	if len(req.Refs) != 2 || req.Refs[1] != "op://v/i/s/f?ssh-format=openssh" {
		t.Fatalf("query params must survive decoding, got %+v", req.Refs)
	}
}

func TestInitOKEncodeExact(t *testing.T) {
	raw, err := json.Marshal(initOK{Type: respInitOK, ID: 1, Protocol: 1, BridgeVersion: "1.2.3"})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	want := `{"type":"init-ok","id":1,"protocol":1,"bridge_version":"1.2.3"}`
	if string(raw) != want {
		t.Fatalf("got %s, want %s", raw, want)
	}
}

func TestResolveResultEncodeOmitsEmpty(t *testing.T) {
	secret := "s"
	raw, err := json.Marshal(resolveResult{Secret: &secret})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if string(raw) != `{"secret":"s"}` {
		t.Fatalf("error field must be omitted when nil, got %s", raw)
	}
	raw, err = json.Marshal(resolveResult{Error: &bridgeError{Code: codeNotFound, Message: "m"}})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if string(raw) != `{"error":{"code":"not_found","message":"m"}}` {
		t.Fatalf("secret field must be omitted when nil, got %s", raw)
	}
}
