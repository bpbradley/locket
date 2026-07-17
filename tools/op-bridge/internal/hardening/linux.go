//go:build linux

package hardening

import "golang.org/x/sys/unix"

// The bridge holds the service account token and resolved secrets in
// memory. Blocking core dumps and marking the process non-dumpable
// (which also blocks same-UID ptrace) keeps them out of reach.
func Harden() {
	_ = unix.Setrlimit(unix.RLIMIT_CORE, &unix.Rlimit{Cur: 0, Max: 0})
	_ = unix.Prctl(unix.PR_SET_DUMPABLE, 0, 0, 0, 0)
}
