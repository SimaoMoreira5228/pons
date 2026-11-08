package main

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
)

const protocolVersion = 3

type emitRequest struct {
	Protocol        int                        `json:"protocol"`
	SourceIRVersion string                     `json:"source_ir_version"`
	Mode            string                     `json:"mode"`
	IR              json.RawMessage            `json:"ir"`
	Options         map[string]json.RawMessage `json:"options"`
	Target          string                     `json:"target"`
}

type emitResponse struct {
	Protocol    int                `json:"protocol"`
	Files       []generatedFile    `json:"files,omitempty"`
	Diagnostics []pluginDiagnostic `json:"diagnostics,omitempty"`
	Coverage    *coverage          `json:"coverage,omitempty"`
	Error       string             `json:"error,omitempty"`
}

type generatedFile struct {
	Path     string `json:"path"`
	Content  string `json:"content"`
	Encoding string `json:"encoding"`
}

type pluginDiagnostic struct {
	Severity string `json:"severity"`
	Code     string `json:"code"`
	Message  string `json:"message"`
	Origin   string `json:"origin,omitempty"`
}

type coverage struct {
	Exact       []string `json:"exact"`
	Opaque      []string `json:"opaque"`
	Lossy       []string `json:"lossy"`
	Unsupported []string `json:"unsupported"`
}

func exactCoverage(document *document) coverage {
	subjects := make([]string, 0, len(document.Types)+len(document.Operations))
	for id := range document.Types {
		subjects = append(subjects, "types."+id)
	}
	for index := range document.Operations {
		subjects = append(subjects, "operations."+operationPath(document.Operations[index].ID))
	}
	sort.Strings(subjects)
	return coverage{Exact: subjects, Opaque: []string{}, Lossy: []string{}, Unsupported: []string{}}
}

func coverageFor(document *document, analysis *analysis) coverage {
	report := exactCoverage(document)
	if len(analysis.lossy) == 0 {
		return report
	}
	lost := map[string]bool{}
	for _, entry := range analysis.lossy {
		lost[entry.subject] = true
	}
	exact := make([]string, 0, len(report.Exact))
	for _, subject := range report.Exact {
		if !lost[subject] {
			exact = append(exact, subject)
		}
	}
	lossy := make([]string, 0, len(lost))
	for subject := range lost {
		lossy = append(lossy, subject)
	}
	sort.Strings(lossy)
	report.Exact = exact
	report.Lossy = lossy
	return report
}

func successResponse(files []generatedFile, document *document, analysis *analysis) emitResponse {
	report := coverageFor(document, analysis)
	diagnostics := make([]pluginDiagnostic, 0, len(analysis.lossy))
	for _, entry := range analysis.lossy {
		diagnostics = append(diagnostics, pluginDiagnostic{
			Severity: "warning",
			Code:     "GO-W001",
			Message:  entry.message,
		})
	}
	return emitResponse{
		Protocol:    protocolVersion,
		Files:       files,
		Diagnostics: diagnostics,
		Coverage:    &report,
	}
}

func (a *analysis) lossyProblems() []problem {
	problems := make([]problem, 0, len(a.lossy))
	for _, entry := range a.lossy {
		problems = append(problems, problem{
			code:    "GO-E003",
			message: entry.message,
			origin:  originOf(entry.subject),
		})
	}
	return problems
}

func failureResponse(problems []problem) emitResponse {
	diagnostics := make([]pluginDiagnostic, 0, len(problems))
	messages := make([]string, 0, len(problems))
	for _, problem := range problems {
		diagnostics = append(diagnostics, pluginDiagnostic{
			Severity: "error",
			Code:     problem.code,
			Message:  problem.message,
			Origin:   problem.origin,
		})
		messages = append(messages, problem.message)
	}
	return emitResponse{
		Protocol:    protocolVersion,
		Diagnostics: diagnostics,
		Error:       strings.Join(messages, "; "),
	}
}

func requestFailure(format string, args ...any) emitResponse {
	return failureResponse([]problem{{code: "GO-E001", message: fmt.Sprintf(format, args...)}})
}

func packageOption(options map[string]json.RawMessage) (string, error) {
	name := "client"
	for _, key := range sortedKeys(options) {
		raw := options[key]
		switch key {
		case "package":
			if err := json.Unmarshal(raw, &name); err != nil {
				return "", fmt.Errorf("option \"package\" is not a string")
			}
		case "validation":
			enabled, err := validationRequested(raw)
			if err != nil {
				return "", err
			}
			if enabled {
				return "", fmt.Errorf("this emitter does not implement response validation")
			}
		default:
			return "", fmt.Errorf("option %q is not one of the options this emitter declares: package", key)
		}
	}
	if err := validatePackageName(name); err != nil {
		return "", err
	}
	return name, nil
}

func validationRequested(raw json.RawMessage) (bool, error) {
	trimmed := strings.TrimSpace(string(raw))
	switch trimmed {
	case "", "null", "false", "\"none\"":
		return false, nil
	}
	if strings.HasPrefix(trimmed, "{") {
		var table struct {
			Backend string `json:"backend"`
		}
		if err := json.Unmarshal(raw, &table); err != nil {
			return false, fmt.Errorf("option \"validation\" is not a validation table")
		}
		return table.Backend != "none", nil
	}
	return true, nil
}
