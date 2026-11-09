package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
)

type document struct {
	IRVersion    string                     `json:"ir_version"`
	Info         info                       `json:"info"`
	Types        map[string]typeDef         `json:"types"`
	AuthSchemes  map[string]authScheme      `json:"auth_schemes"`
	CommonErrors []errorResponse            `json:"common_errors"`
	Operations   []operation                `json:"operations"`
	Ext          map[string]json.RawMessage `json:"ext"`
	Diagnostics  []diagnostic               `json:"diagnostics"`
}

type info struct {
	Title       string                     `json:"title"`
	Version     string                     `json:"version"`
	Description *string                    `json:"description"`
	Servers     []string                   `json:"servers"`
	Ext         map[string]json.RawMessage `json:"ext"`
}

type typeDef struct {
	Name        []string                   `json:"name"`
	Direction   string                     `json:"direction"`
	Kind        string                     `json:"kind"`
	Fields      []field                    `json:"fields"`
	Flattened   []typeRef                  `json:"flattened"`
	Openness    json.RawMessage            `json:"openness"`
	Repr        *enumRepr                  `json:"repr"`
	Variants    []variant                  `json:"variants"`
	Inner       *typeRef                   `json:"inner"`
	Origin      *string                    `json:"origin"`
	Doc         *doc                       `json:"doc"`
	Diagnostics []diagnostic               `json:"diagnostics"`
	Ext         map[string]json.RawMessage `json:"ext"`
}

type field struct {
	Wire        string                     `json:"wire"`
	Ident       []string                   `json:"ident"`
	Ty          typeRef                    `json:"ty"`
	Presence    string                     `json:"presence"`
	Default     json.RawMessage            `json:"default"`
	Access      *string                    `json:"access"`
	Doc         *doc                       `json:"doc"`
	Diagnostics []diagnostic               `json:"diagnostics"`
	Ext         map[string]json.RawMessage `json:"ext"`
}

type enumRepr struct {
	Kind    string `json:"kind"`
	Tag     string `json:"tag"`
	Content string `json:"content"`
}

type variant struct {
	Wire        string                     `json:"wire"`
	Ident       []string                   `json:"ident"`
	Payload     variantPayload             `json:"payload"`
	Doc         *doc                       `json:"doc"`
	Diagnostics []diagnostic               `json:"diagnostics"`
	Ext         map[string]json.RawMessage `json:"ext"`
}

type variantPayload struct {
	Kind   string    `json:"kind"`
	Ty     *typeRef  `json:"ty"`
	Items  []typeRef `json:"items"`
	Fields []field   `json:"fields"`
}

type typeRef struct {
	ID      string
	Variant string
	Inline  *inlineType
	Named   bool
}

type namedRef struct {
	ID      string  `json:"ref"`
	Variant *string `json:"variant"`
}

type inlineType struct {
	Kind             string                     `json:"kind"`
	Bits             uint16                     `json:"bits"`
	Signed           bool                       `json:"signed"`
	Wire             string                     `json:"wire"`
	Minimum          json.RawMessage            `json:"minimum"`
	Maximum          json.RawMessage            `json:"maximum"`
	ExclusiveMinimum json.RawMessage            `json:"exclusive_minimum"`
	ExclusiveMaximum json.RawMessage            `json:"exclusive_maximum"`
	MultipleOf       json.RawMessage            `json:"multiple_of"`
	Format           string                     `json:"format"`
	MinLength        *uint64                    `json:"min_length"`
	MaxLength        *uint64                    `json:"max_length"`
	Pattern          *string                    `json:"pattern"`
	Encoding         string                     `json:"encoding"`
	Codec            string                     `json:"codec"`
	Inner            *typeRef                   `json:"inner"`
	Item             *typeRef                   `json:"item"`
	Unique           *bool                      `json:"unique"`
	Key              *typeRef                   `json:"key"`
	Value            *typeRef                   `json:"value"`
	Items            []typeRef                  `json:"items"`
	Fields           []field                    `json:"fields"`
	Flattened        []typeRef                  `json:"flattened"`
	Openness         json.RawMessage            `json:"openness"`
	Ext              map[string]json.RawMessage `json:"ext"`
}

type pathSegment struct {
	Lit   *string
	Param *string
	Ty    *typeRef
}

type headerParameter struct {
	Wire     string                     `json:"wire"`
	Ident    []string                   `json:"ident"`
	Ty       typeRef                    `json:"ty"`
	Presence string                     `json:"presence"`
	Ext      map[string]json.RawMessage `json:"ext"`
}

type mediaDescriptor struct {
	Type       string            `json:"type"`
	Subtype    string            `json:"subtype"`
	Parameters map[string]string `json:"parameters"`
	Codec      *string           `json:"codec"`
}

type body struct {
	Ty       typeRef                    `json:"ty"`
	Content  string                     `json:"content"`
	Required bool                       `json:"required"`
	Media    *mediaDescriptor           `json:"media"`
	Ext      map[string]json.RawMessage `json:"ext"`
}

type response struct {
	Status  httpStatus                 `json:"status"`
	Content string                     `json:"content"`
	Media   *mediaDescriptor           `json:"media"`
	Body    *typeRef                   `json:"body"`
	Headers []responseHeader           `json:"headers"`
	Ext     map[string]json.RawMessage `json:"ext"`
}

type responseHeader struct {
	Wire     string                     `json:"wire"`
	Ty       typeRef                    `json:"ty"`
	Presence string                     `json:"presence"`
	Ext      map[string]json.RawMessage `json:"ext"`
}

type errorResponse struct {
	Status    httpStatus                 `json:"status"`
	Name      []string                   `json:"name"`
	Content   string                     `json:"content"`
	Media     *mediaDescriptor           `json:"media"`
	Body      *typeRef                   `json:"body"`
	Headers   []responseHeader           `json:"headers"`
	Retryable *bool                      `json:"retryable"`
	Doc       *doc                       `json:"doc"`
	Ext       map[string]json.RawMessage `json:"ext"`
}

type operation struct {
	ID            [][]string                 `json:"id"`
	Method        string                     `json:"method"`
	Path          []pathSegment              `json:"path"`
	Query         *typeRef                   `json:"query"`
	Headers       []headerParameter          `json:"headers"`
	Body          *body                      `json:"body"`
	Responses     []response                 `json:"responses"`
	Errors        []errorResponse            `json:"errors"`
	ErrorsInherit *bool                      `json:"errors_inherit"`
	Stream        json.RawMessage            `json:"stream"`
	Auth          authRequirement            `json:"auth"`
	Provides      json.RawMessage            `json:"provides"`
	Semantics     operationSemantics         `json:"semantics"`
	Doc           *doc                       `json:"doc"`
	Origin        json.RawMessage            `json:"origin"`
	Ext           map[string]json.RawMessage `json:"ext"`
	Diagnostics   []diagnostic               `json:"diagnostics"`
}

type authRequirement struct {
	Alternatives [][]string                 `json:"alternatives"`
	Scopes       map[string][]string        `json:"scopes"`
	Optional     bool                       `json:"optional"`
	Ext          map[string]json.RawMessage `json:"ext"`
}

type authScheme struct {
	Kind        string                     `json:"kind"`
	Format      *string                    `json:"format"`
	In          string                     `json:"in"`
	Name        string                     `json:"name"`
	Cookie      string                     `json:"cookie"`
	Flows       []json.RawMessage          `json:"flows"`
	Description string                     `json:"description"`
	Adapter     *string                    `json:"adapter"`
	Ext         map[string]json.RawMessage `json:"ext"`
}

type operationSemantics struct {
	Safe       bool        `json:"safe"`
	Idempotent bool        `json:"idempotent"`
	Pagination *pagination `json:"pagination"`
}

type pagination struct {
	Request  paginationRequest          `json:"request"`
	Items    string                     `json:"items"`
	Next     *string                    `json:"next"`
	PageSize *pageSize                  `json:"page_size"`
	Ext      map[string]json.RawMessage `json:"ext"`
}

type paginationRequest struct {
	Kind  string `json:"kind"`
	Field string `json:"field"`
}

type pageSize struct {
	Field   string  `json:"field"`
	Default *uint32 `json:"default"`
	Maximum *uint32 `json:"maximum"`
}

type doc struct {
	Summary     *string  `json:"summary"`
	Description *string  `json:"description"`
	Deprecated  *bool    `json:"deprecated"`
	Tags        []string `json:"tags"`
}

type diagnostic struct {
	Severity string                     `json:"severity"`
	Code     string                     `json:"code"`
	Message  string                     `json:"message"`
	Origin   json.RawMessage            `json:"origin"`
	Ext      map[string]json.RawMessage `json:"ext"`
}

type httpStatus struct {
	Code    int
	Class   int
	Default bool
	Unknown bool
}

func decodeDocument(raw json.RawMessage) (*document, error) {
	var version struct {
		IRVersion string `json:"ir_version"`
	}
	if err := json.Unmarshal(raw, &version); err != nil {
		return nil, fmt.Errorf("cannot read the IR envelope: %w", err)
	}
	if version.IRVersion != "0.3" {
		return nil, fmt.Errorf("unsupported IR version %q; this emitter reads 0.3", version.IRVersion)
	}
	var decoded document
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&decoded); err != nil {
		return nil, fmt.Errorf("cannot read the IR document: %w", err)
	}
	return &decoded, nil
}

func (r *typeRef) UnmarshalJSON(raw []byte) error {
	var probe struct {
		Ref  *string `json:"ref"`
		Kind *string `json:"kind"`
	}
	if err := json.Unmarshal(raw, &probe); err != nil {
		return err
	}
	switch {
	case probe.Ref != nil:
		var named namedRef
		if err := decodeStrict(raw, &named); err != nil {
			return err
		}
		r.ID = named.ID
		if named.Variant != nil {
			r.Variant = *named.Variant
		}
		r.Named = true
		return nil
	case probe.Kind != nil:
		var inline inlineType
		if err := decodeStrict(raw, &inline); err != nil {
			return err
		}
		r.Inline = &inline
		return nil
	default:
		return fmt.Errorf("a type reference needs \"ref\" or \"kind\"")
	}
}

func (s *pathSegment) UnmarshalJSON(raw []byte) error {
	var decoded struct {
		Lit   *string  `json:"lit"`
		Param *string  `json:"param"`
		Ty    *typeRef `json:"ty"`
	}
	if err := decodeStrict(raw, &decoded); err != nil {
		return err
	}
	if decoded.Lit != nil {
		s.Lit = decoded.Lit
		return nil
	}
	if decoded.Param != nil && decoded.Ty != nil {
		s.Param = decoded.Param
		s.Ty = decoded.Ty
		return nil
	}
	return fmt.Errorf("a path segment needs \"lit\" or \"param\" with a type")
}

func (s *httpStatus) UnmarshalJSON(raw []byte) error {
	var number int
	if err := json.Unmarshal(raw, &number); err == nil {
		if number < 100 || number > 599 {
			return fmt.Errorf("HTTP status %d is out of range", number)
		}
		s.Code = number
		return nil
	}
	var word string
	if err := json.Unmarshal(raw, &word); err != nil {
		return fmt.Errorf("HTTP status is not a number or a status word")
	}
	switch word {
	case "default":
		s.Default = true
	case "unknown":
		s.Unknown = true
	case "1xx", "2xx", "3xx", "4xx", "5xx":
		s.Class = int(word[0] - '0')
	default:
		return fmt.Errorf("invalid HTTP status %q", word)
	}
	return nil
}

func decodeStrict(raw []byte, target any) error {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	return decoder.Decode(target)
}

func (o *operation) inheritsErrors() bool {
	return o.ErrorsInherit == nil || *o.ErrorsInherit
}

func rawPresent(raw json.RawMessage) bool {
	trimmed := strings.TrimSpace(string(raw))
	return trimmed != "" && trimmed != "null"
}

func operationPath(id [][]string) string {
	segments := make([]string, 0, len(id))
	for _, words := range id {
		escaped := make([]string, 0, len(words))
		for _, word := range words {
			replaced := strings.ReplaceAll(word, "%", "%25")
			replaced = strings.ReplaceAll(replaced, ".", "%2E")
			replaced = strings.ReplaceAll(replaced, "_", "%5F")
			escaped = append(escaped, replaced)
		}
		segments = append(segments, strings.Join(escaped, "_"))
	}
	return strings.Join(segments, ".")
}
