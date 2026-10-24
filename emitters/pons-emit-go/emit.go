package main

import (
	"encoding/json"
	"fmt"
	"go/format"
	"io"
	"unicode"
)

func emit(stdin io.Reader, stdout io.Writer) error {
	var request emitRequest
	decoder := json.NewDecoder(stdin)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&request); err != nil {
		return writeJSON(stdout, requestFailure("cannot read the emit request: %v", err))
	}
	if request.Protocol != protocolVersion {
		return writeJSON(stdout, requestFailure("the request uses protocol %d; this emitter speaks protocol %d", request.Protocol, protocolVersion))
	}
	if request.Mode != "strict" && request.Mode != "lossy" {
		return writeJSON(stdout, requestFailure("the request mode %q is not strict or lossy", request.Mode))
	}
	if request.Target != "go" {
		return writeJSON(stdout, requestFailure("this emitter is named go, but the request targets %q", request.Target))
	}
	pkg, err := packageOption(request.Options)
	if err != nil {
		return writeJSON(stdout, requestFailure("%v", err))
	}
	document, err := decodeDocument(request.IR)
	if err != nil {
		return writeJSON(stdout, requestFailure("%v", err))
	}
	analysis := newAnalysis(document, pkg)
	if len(analysis.problems) > 0 {
		return writeJSON(stdout, failureResponse(analysis.problems))
	}
	files, err := generatedFiles(analysis)
	if err != nil {
		return writeJSON(stdout, requestFailure("%v", err))
	}
	return writeJSON(stdout, successResponse(files, document))
}

func validatePackageName(name string) error {
	if name == "" || name == "_" {
		return fmt.Errorf("the package option must be a Go identifier")
	}
	for index, character := range name {
		if index == 0 {
			if character != '_' && !unicode.IsLetter(character) {
				return fmt.Errorf("the package option %q is not a Go identifier", name)
			}
			continue
		}
		if character != '_' && !unicode.IsLetter(character) && !unicode.IsDigit(character) {
			return fmt.Errorf("the package option %q is not a Go identifier", name)
		}
	}
	if goKeywords[name] || predeclared[name] {
		return fmt.Errorf("the package option %q is a reserved Go word", name)
	}
	if name == "main" {
		return fmt.Errorf("the package option cannot be main")
	}
	return nil
}

func generatedFiles(analysis *analysis) ([]generatedFile, error) {
	rendered := []struct {
		path    string
		content string
	}{
		{path: "doc.go", content: renderDoc(analysis)},
		{path: "types.go", content: renderTypes(analysis)},
		{path: "errors.go", content: renderErrors(analysis)},
		{path: "client.go", content: renderClient(analysis)},
		{path: "operations.go", content: renderOperations(analysis)},
	}
	files := make([]generatedFile, 0, len(rendered))
	for _, file := range rendered {
		formatted, err := format.Source([]byte(file.content))
		if err != nil {
			return nil, fmt.Errorf("generated %s is not valid Go: %w", file.path, err)
		}
		files = append(files, generatedFile{Path: file.path, Content: string(formatted), Encoding: "utf8"})
	}
	return files, nil
}
