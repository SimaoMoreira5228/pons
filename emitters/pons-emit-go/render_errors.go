package main

import (
	"fmt"
	"strings"
)

func renderErrors(a *analysis) string {
	var builder strings.Builder
	writeFileHeader(&builder, a.pkg, []string{"encoding/json", "fmt", "net/http"})
	builder.WriteString(`
type ConfigurationError struct {
	Message string
}

func (e *ConfigurationError) Error() string {
	return "pons: " + e.Message
}

type APIError struct {
	StatusCode int
	Status     string
	Header     http.Header
	Body       json.RawMessage
}

func (e *APIError) Error() string {
	if len(e.Body) == 0 {
		return fmt.Sprintf("pons: unexpected status %s", e.Status)
	}
	return fmt.Sprintf("pons: unexpected status %s: %s", e.Status, e.Body)
}

func (e *APIError) DecodeBody(target any) error {
	if len(e.Body) == 0 {
		return nil
	}
	return json.Unmarshal(e.Body, target)
}

type DecodeError struct {
	StatusCode int
	Body       json.RawMessage
	Err        error
}

func (e *DecodeError) Error() string {
	return fmt.Sprintf("pons: cannot decode response %d: %v", e.StatusCode, e.Err)
}

func (e *DecodeError) Unwrap() error {
	return e.Err
}
`)

	for _, key := range a.errorOrder {
		def := a.errorDefs[key]
		bodyType := errorBodyType(def)
		builder.WriteString("\n")
		fmt.Fprintf(&builder, "type %s struct {\n", def.Name)
		builder.WriteString("\tStatusCode int\n")
		builder.WriteString("\tHeader     http.Header\n")
		if bodyType != "" {
			fmt.Fprintf(&builder, "\tBody       %s\n", bodyType)
		}
		builder.WriteString("}\n\n")
		fmt.Fprintf(&builder, "func (e *%s) Error() string {\n", def.Name)
		fmt.Fprintf(&builder, "\treturn fmt.Sprintf(\"pons: %%s (%%d)\", %q, e.StatusCode)\n", def.Message)
		builder.WriteString("}\n")
	}
	return builder.String()
}

func errorBodyType(def *errorType) string {
	switch def.Content {
	case "json":
		if def.Body == nil {
			return ""
		}
		if def.Body.Named {
			return "*" + def.BodyGo
		}
		return def.BodyGo
	case "text":
		return "string"
	default:
		return ""
	}
}
