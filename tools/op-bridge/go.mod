module github.com/bpbradley/locket/tools/op-bridge

go 1.26.5

require (
	// Pinned to a main branch commit for now. every tagged release through
	// v0.4.1-beta.1 fails to compile with CGO_ENABLED=0.
	// Main replaces the guard with a runtime error.
	// Move to the next tagged release once available.
	github.com/1password/onepassword-sdk-go v0.4.1-0.20260605221002-f1117e36ce06
	golang.org/x/sys v0.47.0
)

require (
	github.com/dylibso/observe-sdk/go v0.0.0-20240828172851-9145d8ad07e1 // indirect
	github.com/extism/go-sdk v1.7.1 // indirect
	github.com/gobwas/glob v0.2.3 // indirect
	github.com/ianlancetaylor/demangle v0.0.0-20251118225945-96ee0021ea0f // indirect
	github.com/tetratelabs/wabin v0.0.0-20230304001439-f6f874872834 // indirect
	github.com/tetratelabs/wazero v1.11.0 // indirect
	go.opentelemetry.io/proto/otlp v1.9.0 // indirect
	google.golang.org/protobuf v1.36.11 // indirect
)
