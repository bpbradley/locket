package main

import (
	"context"
	"log"
	"os"

	"github.com/1password/onepassword-sdk-go"

	"github.com/bpbradley/locket/tools/op-bridge/internal/hardening"
)

// Stamped at build time: -ldflags "-X main.version=<version>"
var version = "dev"

func main() {
	log.SetFlags(0)
	log.SetOutput(os.Stderr)
	hardening.Harden()

	srv := newServer(sdkClient, os.Stdout)
	srv.run(context.Background(), os.Stdin)
}

func sdkClient(ctx context.Context, token string) (resolver, error) {
	client, err := onepassword.NewClient(ctx,
		onepassword.WithServiceAccountToken(token),
		onepassword.WithIntegrationInfo("locket", version),
	)
	if err != nil {
		return nil, err
	}
	return client.Secrets(), nil
}
