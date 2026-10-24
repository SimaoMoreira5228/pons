package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
)

func main() {
	if err := run(os.Args[1:], os.Stdin, os.Stdout, os.Stderr); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run(args []string, stdin io.Reader, stdout, stderr io.Writer) error {
	if len(args) != 1 {
		return fmt.Errorf("usage: pons-emit-go <describe|emit>")
	}
	switch args[0] {
	case "describe":
		return writeJSON(stdout, buildManifest())
	case "emit":
		return emit(stdin, stdout)
	default:
		return fmt.Errorf("unknown command %q; use describe or emit", args[0])
	}
}

func writeJSON(w io.Writer, value any) error {
	encoder := json.NewEncoder(w)
	encoder.SetIndent("", "  ")
	return encoder.Encode(value)
}
