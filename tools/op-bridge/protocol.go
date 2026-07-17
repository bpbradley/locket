// locket-op-bridge speaks JSON lines with locket over stdin/stdout.
// Protocol v1: an `init` message (carrying the service account token)
// must arrive first, then any number of `resolve` batches. Responses
// are correlated by id. The bridge exits when stdin reaches EOF.
package main

const protocolVersion = 1

type requestType string

const (
	reqInit    requestType = "init"
	reqResolve requestType = "resolve"
)

type responseType string

const (
	respInitOK    responseType = "init-ok"
	respResolveOK responseType = "resolve-ok"
	respError     responseType = "error"
)

type errorCode string

const (
	codeNotFound            errorCode = "not_found"
	codeRateLimited         errorCode = "rate_limited"
	codeInvalidReference    errorCode = "invalid_reference"
	codeUnsupportedProtocol errorCode = "unsupported_protocol"
	codeBadRequest          errorCode = "bad_request"
	codeInternal            errorCode = "internal"
	codeOther               errorCode = "other"
)

// Requests arrive as one flat JSON object. The envelope carries the
// variant tag and correlation id; per-variant payloads are decoded
// separately so each handler only sees the fields its variant has.
type envelope struct {
	Type requestType `json:"type"`
	ID   uint64      `json:"id"`
}

type initRequest struct {
	Protocol int    `json:"protocol"`
	Token    string `json:"token"`
}

type resolveRequest struct {
	Refs []string `json:"refs"`
}

type initOK struct {
	Type          responseType `json:"type"`
	ID            uint64       `json:"id"`
	Protocol      int          `json:"protocol"`
	BridgeVersion string       `json:"bridge_version"`
}

type resolveOK struct {
	Type    responseType             `json:"type"`
	ID      uint64                   `json:"id"`
	Results map[string]resolveResult `json:"results"`
}

type resolveResult struct {
	Secret *string      `json:"secret,omitempty"`
	Error  *bridgeError `json:"error,omitempty"`
}

type bridgeError struct {
	Code    errorCode `json:"code"`
	Message string    `json:"message"`
}

type errorResponse struct {
	Type    responseType `json:"type"`
	ID      uint64       `json:"id"`
	Code    errorCode    `json:"code"`
	Message string       `json:"message"`
}
