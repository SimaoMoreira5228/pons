package main

import (
	"bytes"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

func fixtureDocument(t *testing.T) []byte {
	t.Helper()
	raw, err := os.ReadFile("testdata/contract.json")
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func decodeFixture(t *testing.T) *document {
	t.Helper()
	doc, err := decodeDocument(fixtureDocument(t))
	if err != nil {
		t.Fatal(err)
	}
	return doc
}

func emitFixture(t *testing.T, raw []byte, options map[string]any) emitResponse {
	t.Helper()
	return emitFixtureMode(t, raw, options, "strict")
}

func emitFixtureMode(t *testing.T, raw []byte, options map[string]any, mode string) emitResponse {
	t.Helper()
	request := map[string]any{
		"protocol":          protocolVersion,
		"source_ir_version": "0.3",
		"mode":              mode,
		"ir":                json.RawMessage(raw),
		"options":           options,
		"target":            "go",
	}
	encoded, err := json.Marshal(request)
	if err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	if err := emit(bytes.NewReader(encoded), &output); err != nil {
		t.Fatal(err)
	}
	var response emitResponse
	if err := json.Unmarshal(output.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	return response
}

func invokeResponse(t *testing.T, response emitResponse, path string) string {
	t.Helper()
	for _, file := range response.Files {
		if file.Path == path {
			return file.Content
		}
	}
	t.Fatalf("emit returned no %s", path)
	return ""
}

func mutateFixture(t *testing.T) map[string]any {
	t.Helper()
	var value map[string]any
	if err := json.Unmarshal(fixtureDocument(t), &value); err != nil {
		t.Fatal(err)
	}
	return value
}

func emitMutated(t *testing.T, value map[string]any) emitResponse {
	t.Helper()
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return emitFixture(t, raw, map[string]any{"package": "fixture"})
}

func TestFixtureEmitsEverySubjectExactly(t *testing.T) {
	response := emitFixture(t, fixtureDocument(t), map[string]any{"package": "fixture"})
	if response.Error != "" {
		t.Fatalf("emit failed: %s", response.Error)
	}
	if response.Coverage == nil {
		t.Fatal("emit returned no coverage")
	}
	if len(response.Coverage.Opaque) != 0 || len(response.Coverage.Lossy) != 0 || len(response.Coverage.Unsupported) != 0 {
		t.Fatalf("coverage is not exact: %#v", response.Coverage)
	}
	doc := decodeFixture(t)
	if len(response.Coverage.Exact) != len(doc.Types)+len(doc.Operations) {
		t.Fatalf("coverage = %v", response.Coverage.Exact)
	}
	if len(response.Files) != 5 {
		t.Fatalf("files = %d", len(response.Files))
	}
}

func TestCursorPaginationEmitsAnIterator(t *testing.T) {
	response := emitFixture(t, fixtureDocument(t), map[string]any{"package": "fixture"})
	if response.Error != "" {
		t.Fatalf("emit failed: %s", response.Error)
	}
	operations := invokeResponse(t, response, "operations.go")
	want := "func (c *Client) EventsListAll(ctx context.Context, query EventQuery, yield func(Event) error) error"
	if !bytes.Contains([]byte(operations), []byte(want)) {
		t.Fatalf("operations.go does not declare the iterator:\n%s", operations)
	}
	if bytes.Contains([]byte(operations), []byte("go func")) || bytes.Contains([]byte(operations), []byte("chan ")) {
		t.Fatalf("the iterator starts a goroutine or channel:\n%s", operations)
	}
}

func TestOffsetPaginationIsLossy(t *testing.T) {
	raw, err := os.ReadFile("testdata/offset.json")
	if err != nil {
		t.Fatal(err)
	}
	strict := emitFixture(t, raw, map[string]any{"package": "fixture"})
	if strict.Error == "" || !bytes.Contains([]byte(strict.Error), []byte("offset pagination")) {
		t.Fatalf("strict response = %#v", strict)
	}
	lossy := emitFixtureMode(t, raw, map[string]any{"package": "fixture"}, "lossy")
	if lossy.Error != "" {
		t.Fatalf("lossy emit failed: %s", lossy.Error)
	}
	if lossy.Coverage == nil || len(lossy.Coverage.Lossy) != 1 || lossy.Coverage.Lossy[0] != "operations.events.list" {
		t.Fatalf("coverage = %#v", lossy.Coverage)
	}
	if operations := invokeResponse(t, lossy, "operations.go"); bytes.Contains([]byte(operations), []byte("EventsListAll(")) {
		t.Fatalf("offset pagination emitted an iterator:\n%s", operations)
	}
	if len(lossy.Diagnostics) != 1 || !bytes.Contains([]byte(lossy.Diagnostics[0].Message), []byte("operations.events.list")) {
		t.Fatalf("diagnostics = %#v", lossy.Diagnostics)
	}
}

func TestIteratorNameCollisionIsAnError(t *testing.T) {
	value := mutateFixture(t)
	operations := value["operations"].([]any)
	operations[0].(map[string]any)["id"] = []any{[]any{"events"}, []any{"list"}, []any{"all"}}
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	response := emitFixtureMode(t, raw, map[string]any{"package": "fixture"}, "lossy")
	if response.Error == "" {
		t.Fatalf("emit accepted the collision: %#v", response)
	}
	for _, want := range []string{"EventsListAll", "operations.events.list", "operations.events.list.all"} {
		if !bytes.Contains([]byte(response.Error), []byte(want)) {
			t.Fatalf("error %q does not name %q", response.Error, want)
		}
	}
}

func TestUnresolvedPaginationPointerIsLossy(t *testing.T) {
	value := mutateFixture(t)
	operations := value["operations"].([]any)
	semantics := operations[3].(map[string]any)["semantics"].(map[string]any)
	pagination := semantics["pagination"].(map[string]any)
	pagination["items"] = "/missing"
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	strict := emitFixture(t, raw, map[string]any{"package": "fixture"})
	if strict.Error == "" || !bytes.Contains([]byte(strict.Error), []byte("operations.events.list")) || !bytes.Contains([]byte(strict.Error), []byte(`"/missing"`)) {
		t.Fatalf("strict response = %#v", strict)
	}
	lossy := emitFixtureMode(t, raw, map[string]any{"package": "fixture"}, "lossy")
	if lossy.Error != "" {
		t.Fatalf("lossy emit failed: %s", lossy.Error)
	}
	if lossy.Coverage == nil || len(lossy.Coverage.Lossy) != 1 || lossy.Coverage.Lossy[0] != "operations.events.list" {
		t.Fatalf("coverage = %#v", lossy.Coverage)
	}
	if emitted := invokeResponse(t, lossy, "operations.go"); bytes.Contains([]byte(emitted), []byte("EventsListAll(")) {
		t.Fatalf("the unresolved pointer emitted an iterator:\n%s", emitted)
	}
}

func TestTypesMapTheWireShapes(t *testing.T) {
	doc := decodeFixture(t)
	analysis := newAnalysis(doc, "fixture")
	if len(analysis.problems) > 0 {
		t.Fatalf("analysis problems: %#v", analysis.problems)
	}
	user := analysis.typeByID["fixture::User"]
	if user == nil {
		t.Fatal("User plan is missing")
	}
	fields := map[string]fieldPlan{}
	for _, field := range user.Fields {
		fields[field.Wire] = field
	}
	for _, testCase := range []struct {
		wire   string
		goType string
	}{
		{"id", "int64"},
		{"score", "uint64"},
		{"email", "*string"},
		{"nickname", "*string"},
		{"tags", "[]string"},
		{"labels", "map[string]string"},
	} {
		if got := fields[testCase.wire].GoType; got != testCase.goType {
			t.Errorf("%s = %q, want %q", testCase.wire, got, testCase.goType)
		}
	}
	status := analysis.typeByID["fixture::Status"]
	if status == nil || len(status.Consts) != 2 || status.Consts[0].Name != "StatusActive" {
		t.Fatalf("status plan = %#v", status)
	}
	handle := analysis.typeByID["fixture::UserHandle"]
	if handle == nil || handle.Inner != "string" {
		t.Fatalf("handle plan = %#v", handle)
	}
}

func TestInlineStructRendersItsShape(t *testing.T) {
	doc := decodeFixture(t)
	analysis := newAnalysis(doc, "fixture")
	if len(analysis.problems) > 0 {
		t.Fatalf("analysis problems: %#v", analysis.problems)
	}
	event := analysis.typeByID["fixture::Event"]
	var details *fieldPlan
	for index := range event.Fields {
		if event.Fields[index].Wire == "details" {
			details = &event.Fields[index]
		}
	}
	if details == nil {
		t.Fatal("Event details plan is missing")
	}
	shape := details.GoType
	if !strings.HasPrefix(shape, "*struct {\n") {
		t.Fatalf("details = %q", shape)
	}
	for _, want := range []string{
		"Source string `json:\"source\"`",
		"Source2 *string `json:\"origin,omitempty\"`",
		"Readings []struct {\n",
		"Counters map[string]struct {\n",
		"Count int32 `json:\"count\"`",
		"Code string `json:\"code\"`",
		"Message string `json:\"message\"`",
	} {
		if !strings.Contains(shape, want) {
			t.Fatalf("details = %q does not contain %q", shape, want)
		}
	}
}

func inlineMutation(t *testing.T, mutate func(map[string]any)) map[string]any {
	t.Helper()
	value := mutateFixture(t)
	types := value["types"].(map[string]any)
	fields := types["fixture::Event"].(map[string]any)["fields"].([]any)
	for _, raw := range fields {
		field := raw.(map[string]any)
		if field["wire"] == "details" {
			mutate(field["ty"].(map[string]any))
			return value
		}
	}
	t.Fatal("Event details field is missing")
	return nil
}

func TestOpenInlineStructIsLossy(t *testing.T) {
	for _, testCase := range []struct {
		name  string
		value any
	}{
		{"preserve_typed", map[string]any{"preserve_typed": map[string]any{"value": map[string]any{"kind": "string"}}}},
		{"preserve_opaque", "preserve_opaque"},
	} {
		t.Run(testCase.name, func(t *testing.T) {
			value := inlineMutation(t, func(details map[string]any) {
				details["openness"] = testCase.value
			})
			raw, err := json.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
			strict := emitFixture(t, raw, map[string]any{"package": "fixture"})
			if strict.Error == "" || !bytes.Contains([]byte(strict.Error), []byte("object openness")) {
				t.Fatalf("strict response = %#v", strict)
			}
			lossy := emitFixtureMode(t, raw, map[string]any{"package": "fixture"}, "lossy")
			if lossy.Error != "" {
				t.Fatalf("lossy emit failed: %s", lossy.Error)
			}
			if lossy.Coverage == nil || len(lossy.Coverage.Lossy) != 1 || lossy.Coverage.Lossy[0] != "types.fixture::Event" {
				t.Fatalf("coverage = %#v", lossy.Coverage)
			}
			if len(lossy.Diagnostics) != 1 || !bytes.Contains([]byte(lossy.Diagnostics[0].Message), []byte("inline struct")) {
				t.Fatalf("diagnostics = %#v", lossy.Diagnostics)
			}
			if types := invokeResponse(t, lossy, "types.go"); !bytes.Contains([]byte(types), []byte(`json:"readings"`)) {
				t.Fatalf("types.go does not keep the inline shape:\n%s", types)
			}
		})
	}
}

func TestOpenInlineStructInAnOperationIsLossy(t *testing.T) {
	value := mutateFixture(t)
	operations := value["operations"].([]any)
	body := operations[1].(map[string]any)["body"].(map[string]any)
	body["ty"] = map[string]any{
		"kind":     "struct",
		"openness": "preserve_opaque",
		"fields": []any{
			map[string]any{"wire": "name", "ident": []any{"name"}, "ty": map[string]any{"kind": "string"}, "presence": "required"},
		},
	}
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	lossy := emitFixtureMode(t, raw, map[string]any{"package": "fixture"}, "lossy")
	if lossy.Error != "" {
		t.Fatalf("lossy emit failed: %s", lossy.Error)
	}
	if lossy.Coverage == nil || len(lossy.Coverage.Lossy) != 1 || lossy.Coverage.Lossy[0] != "operations.users.create" {
		t.Fatalf("coverage = %#v", lossy.Coverage)
	}
	if operations := invokeResponse(t, lossy, "operations.go"); !bytes.Contains([]byte(operations), []byte("Name string `json:\"name\"`")) {
		t.Fatalf("operations.go does not use the inline body:\n%s", operations)
	}
}

func TestFlattenedInlineMapIsRejected(t *testing.T) {
	value := inlineMutation(t, func(details map[string]any) {
		details["flattened"] = []any{
			map[string]any{"kind": "map", "key": map[string]any{"kind": "string"}, "value": map[string]any{"kind": "string"}},
		}
	})
	response := emitMutated(t, value)
	if response.Error == "" || !bytes.Contains([]byte(response.Error), []byte("flattens")) {
		t.Fatalf("response = %#v", response)
	}
}

func TestUnsupportedConstructsAreRejected(t *testing.T) {
	for _, testCase := range []struct {
		name   string
		change func(map[string]any)
		want   string
	}{
		{
			name: "stream",
			change: func(value map[string]any) {
				operations := value["operations"].([]any)
				operations[0].(map[string]any)["stream"] = map[string]any{"kind": "ndjson", "item": map[string]any{"ref": "fixture::User"}}
			},
			want: "streams",
		},
		{
			name: "internal enum",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				types["fixture::Status"].(map[string]any)["repr"] = map[string]any{"kind": "internal", "tag": "kind"}
			},
			want: "string enums",
		},
		{
			name: "open object",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				types["fixture::User"].(map[string]any)["openness"] = "preserve_opaque"
			},
			want: "object openness",
		},
		{
			name: "flattened struct",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				types["fixture::User"].(map[string]any)["flattened"] = []any{map[string]any{"ref": "fixture::Problem"}}
			},
			want: "flattened",
		},
		{
			name: "opaque value",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				fields := types["fixture::User"].(map[string]any)["fields"].([]any)
				fields[0].(map[string]any)["ty"] = map[string]any{"kind": "opaque", "codec": "custom", "wire": "json"}
			},
			want: "opaque",
		},
		{
			name: "128 bit integer",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				fields := types["fixture::User"].(map[string]any)["fields"].([]any)
				fields[0].(map[string]any)["ty"] = map[string]any{"kind": "int", "bits": 128, "signed": true, "wire": "number"}
			},
			want: "64 bits",
		},
		{
			name: "tuple",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				fields := types["fixture::User"].(map[string]any)["fields"].([]any)
				fields[0].(map[string]any)["ty"] = map[string]any{"kind": "tuple", "items": []any{map[string]any{"kind": "string"}}}
			},
			want: "tuple",
		},
		{
			name: "optional nullable field",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				fields := types["fixture::User"].(map[string]any)["fields"].([]any)
				fields[4].(map[string]any)["ty"] = map[string]any{"kind": "nullable", "inner": map[string]any{"kind": "string"}}
			},
			want: "optional and nullable",
		},
		{
			name: "required value cycle",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				fields := types["fixture::User"].(map[string]any)["fields"].([]any)
				fields[0].(map[string]any)["ty"] = map[string]any{"ref": "fixture::User"}
			},
			want: "required cycle",
		},
		{
			name: "required cycle through an inline struct",
			change: func(value map[string]any) {
				types := value["types"].(map[string]any)
				fields := types["fixture::User"].(map[string]any)["fields"].([]any)
				fields[0].(map[string]any)["ty"] = map[string]any{
					"kind":     "struct",
					"openness": "reject",
					"fields": []any{
						map[string]any{"wire": "self", "ident": []any{"self"}, "ty": map[string]any{"ref": "fixture::User"}, "presence": "required"},
					},
				}
			},
			want: "required cycle",
		},
		{
			name: "custom auth",
			change: func(value map[string]any) {
				schemes := value["auth_schemes"].(map[string]any)
				schemes["custom"] = map[string]any{"kind": "custom", "description": "custom"}
				operations := value["operations"].([]any)
				operations[2].(map[string]any)["auth"] = map[string]any{"alternatives": []any{[]any{"custom"}}, "optional": false}
			},
			want: "custom",
		},
		{
			name: "validation request",
			change: func(map[string]any) {
			},
			want: "validation",
		},
	} {
		t.Run(testCase.name, func(t *testing.T) {
			value := mutateFixture(t)
			testCase.change(value)
			var response emitResponse
			if testCase.name == "validation request" {
				raw, err := json.Marshal(value)
				if err != nil {
					t.Fatal(err)
				}
				response = emitFixture(t, raw, map[string]any{"package": "fixture", "validation": map[string]any{"backend": "zod"}})
			} else {
				response = emitMutated(t, value)
			}
			if response.Error == "" {
				t.Fatalf("emit accepted %s", testCase.name)
			}
			if !bytes.Contains([]byte(response.Error), []byte(testCase.want)) {
				t.Fatalf("error %q does not mention %q", response.Error, testCase.want)
			}
		})
	}
}

func TestUnsupportedIRVersionIsRefused(t *testing.T) {
	value := mutateFixture(t)
	value["ir_version"] = "0.1"
	response := emitMutated(t, value)
	if response.Error == "" || !bytes.Contains([]byte(response.Error), []byte("unsupported IR version")) {
		t.Fatalf("response = %#v", response)
	}
}

func TestBadPackageOptionIsRefused(t *testing.T) {
	response := emitFixture(t, fixtureDocument(t), map[string]any{"package": "not a package"})
	if response.Error == "" || !bytes.Contains([]byte(response.Error), []byte("Go identifier")) {
		t.Fatalf("response = %#v", response)
	}
}
