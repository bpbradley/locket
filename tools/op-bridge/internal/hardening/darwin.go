//go:build darwin

package hardening

import "golang.org/x/sys/unix"

func Harden() {
	_ = unix.Setrlimit(unix.RLIMIT_CORE, &unix.Rlimit{Cur: 0, Max: 0})
}
