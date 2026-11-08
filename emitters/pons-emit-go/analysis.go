package main

import (
	"fmt"
	"sort"
	"strings"
)

type problem struct {
	code    string
	message string
	origin  string
}

type lossyOperation struct {
	subject string
	message string
}

type analysis struct {
	doc          *document
	pkg          string
	packageNames *nameRegistry
	methodNames  *nameRegistry
	methodOwners map[string]string

	typeNames  map[string]string
	types      []*typePlan
	typeByID   map[string]*typePlan
	operations []*operationPlan
	queryTypes map[string]bool

	errorDefs  map[string]*errorType
	errorOrder []string

	bearerSchemes map[string]bool
	apiKeySchemes map[string]bool
	apiKeyHeader  string
	apiKeyQuery   string

	usesJSONBody   bool
	usesPathEscape bool
	usesPayload    bool

	problems []problem
	lossy    []lossyOperation
}

type typePlan struct {
	ID     string
	Name   string
	Kind   string
	Def    typeDef
	Fields []fieldPlan
	Consts []constPlan
	Inner  string
}

type fieldPlan struct {
	Wire     string
	Name     string
	GoType   string
	Tag      string
	Ty       typeRef
	Optional bool
}

type constPlan struct {
	Name string
	Wire string
}

type errorType struct {
	Key     string
	Name    string
	Message string
	Content string
	Body    *typeRef
	BodyGo  string
	Words   []string
	Status  string
	Code    int
}

type operationPlan struct {
	Op         *operation
	Name       string
	Params     []paramPlan
	Path       []pathPiece
	QueryType  string
	QueryParam string
	Body       *bodyPlan
	Success    *successPlan
	ErrorKeys  []string
	Auth       authPlan
	Pagination *paginationPlan
}

type paramPlan struct {
	Kind     string
	Name     string
	GoType   string
	Wire     string
	Optional bool
}

type bodyPlan struct {
	Param    string
	GoType   string
	Optional bool
	Ty       typeRef
}

type successPlan struct {
	Status  int
	Type    *typeRef
	GoType  string
	Pointer bool
}

type authPlan struct {
	Bearer   bool
	APIKey   bool
	Required bool
}

type pathPiece struct {
	Literal string
	Param   string
}

func newAnalysis(doc *document, pkg string) *analysis {
	a := &analysis{
		doc:           doc,
		pkg:           pkg,
		packageNames:  newNameRegistry(),
		methodNames:   newNameRegistry(),
		methodOwners:  map[string]string{},
		typeNames:     map[string]string{},
		typeByID:      map[string]*typePlan{},
		queryTypes:    map[string]bool{},
		errorDefs:     map[string]*errorType{},
		bearerSchemes: map[string]bool{},
		apiKeySchemes: map[string]bool{},
	}
	for _, reserved := range []string{"Client", "ConfigurationError", "APIError", "DecodeError"} {
		a.packageNames.reserve(reserved)
	}
	for _, reserved := range []string{"BaseURL", "HTTPClient", "BearerToken", "APIKey"} {
		a.methodNames.reserve(reserved)
		a.methodOwners[reserved] = "Client." + reserved
	}
	a.assignTypeNames()
	a.buildTypePlans()
	a.checkValueCycles()
	a.buildOperationPlans()
	a.finalize()
	return a
}

func (a *analysis) addProblem(origin, format string, args ...any) {
	a.problems = append(a.problems, problem{
		code:    "GO-E002",
		message: fmt.Sprintf(format, args...),
		origin:  origin,
	})
}

func (a *analysis) markLossy(origin, format string, args ...any) {
	a.markLossySubject("operations."+origin, format, args...)
}

func (a *analysis) markLossySubject(subject, format string, args ...any) {
	a.lossy = append(a.lossy, lossyOperation{
		subject: subject,
		message: fmt.Sprintf(format, args...),
	})
}

func originOf(subject string) string {
	if trimmed, ok := strings.CutPrefix(subject, "types."); ok {
		return trimmed
	}
	return strings.TrimPrefix(subject, "operations.")
}

func (a *analysis) assignTypeNames() {
	groups := map[string][]string{}
	for id, def := range a.doc.Types {
		base := exportedName(def.Name)
		groups[base] = append(groups[base], id)
	}
	bases := make([]string, 0, len(groups))
	for base := range groups {
		bases = append(bases, base)
	}
	sort.Strings(bases)
	for _, base := range bases {
		ids := groups[base]
		sort.Strings(ids)
		if len(ids) == 1 {
			a.typeNames[ids[0]] = a.packageNames.claim(base)
			continue
		}
		for _, id := range ids {
			def := a.doc.Types[id]
			suffix := directionSuffix(def.Direction)
			name := base + suffix
			if suffix == "" {
				name = base + "Both"
			}
			a.typeNames[id] = a.packageNames.claim(name)
		}
	}
}

func directionSuffix(direction string) string {
	switch direction {
	case "input":
		return "Input"
	case "output":
		return "Output"
	default:
		return ""
	}
}

func (a *analysis) finalize() {
	if len(a.bearerSchemes) > 1 {
		a.addProblem("auth_schemes", "the document uses %d bearer schemes; the generated client has one bearer token", len(a.bearerSchemes))
	}
	if len(a.apiKeySchemes) > 1 {
		a.addProblem("auth_schemes", "the document uses %d API key schemes; the generated client has one API key", len(a.apiKeySchemes))
	}
	keys := make([]string, 0, len(a.errorDefs))
	for key := range a.errorDefs {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		def := a.errorDefs[key]
		def.Name = a.packageNames.claim(exportedErrorName(def))
		a.errorOrder = append(a.errorOrder, key)
	}
}

func exportedErrorName(def *errorType) string {
	words := def.Words
	if len(words) == 0 {
		words = []string{"http", def.Status}
	}
	name := exportedName(words)
	if strings.HasSuffix(name, "Error") {
		return name
	}
	return name + "Error"
}

func errorKey(response errorResponse) string {
	var body string
	if response.Body != nil {
		body = refSignature(*response.Body)
	}
	return statusKey(response.Status) + "|" + strings.Join(response.Name, "_") + "|" + response.Content + "|" + body
}

func statusKey(status httpStatus) string {
	switch {
	case status.Code != 0:
		return fmt.Sprintf("%d", status.Code)
	case status.Class != 0:
		return fmt.Sprintf("%dxx", status.Class)
	case status.Default:
		return "default"
	case status.Unknown:
		return "unknown"
	default:
		return "invalid"
	}
}

func statusText(status httpStatus) string {
	return statusKey(status)
}

func sortedKeys[T any](values map[string]T) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}
