#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

(
  cd "$root"
  go test ./...
  go vet ./...
  go build -o "$work/pons-emit-go" .
)

"$work/pons-emit-go" describe > "$work/manifest.json"

python3 - "$work" "$root/testdata/contract.json" "$root/testdata/offset.json" <<'PY'
import json
import pathlib
import subprocess
import sys

work = pathlib.Path(sys.argv[1])
contract_path = pathlib.Path(sys.argv[2])
offset_path = pathlib.Path(sys.argv[3])
contract = json.loads(contract_path.read_text())

manifest = json.loads((work / "manifest.json").read_text())
assert manifest["name"] == "go", manifest
assert manifest["version"] == "0.1.0", manifest
assert manifest["protocol"] == {"min": "3.0", "max": "3.0"}, manifest
assert manifest["ir_version"] == {"min": "0.2", "max": "0.2"}, manifest
capabilities = manifest["capabilities"]
assert capabilities["streaming"] == [], capabilities
assert capabilities["pagination"] == ["pagination.cursor"], capabilities
assert capabilities["enum_reprs"] == ["string"], capabilities
assert capabilities["object_openness"] == ["reject"], capabilities
assert capabilities["wide_integers"] is True, capabilities
for key in ("provides", "custom_auth", "flattened", "opaque"):
    assert capabilities[key] is False, (key, capabilities)
assert manifest["option_schema"]["options"]["package"]["type"] == "string", manifest

request = {
    "protocol": 3,
    "source_ir_version": "0.2",
    "mode": "strict",
    "ir": contract,
    "options": {"package": "fixture"},
    "target": "go",
}
result = subprocess.run(
    [str(work / "pons-emit-go"), "emit"],
    input=json.dumps(request),
    text=True,
    capture_output=True,
)
assert result.returncode == 0, result.stderr
response = json.loads(result.stdout)
assert response.get("error") is None, response
assert response["protocol"] == 3, response

module = work / "module"
module.mkdir()
for file in response["files"]:
    relative = pathlib.PurePosixPath(file["path"])
    assert not relative.is_absolute(), relative
    assert ".." not in relative.parts, relative
    assert file["encoding"] == "utf8", file
    destination = module.joinpath(*relative.parts)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(file["content"])

coverage = response["coverage"]
expected = sorted(
    ["types." + name for name in contract["types"]]
    + ["operations." + ".".join("_".join(part) for part in operation["id"]) for operation in contract["operations"]]
)
assert coverage == {"exact": expected, "opaque": [], "lossy": [], "unsupported": []}, coverage

offset = json.loads(offset_path.read_text())
offset_request = {
    "protocol": 3,
    "source_ir_version": "0.2",
    "mode": "lossy",
    "ir": offset,
    "options": {"package": "fixture"},
    "target": "go",
}
offset_result = subprocess.run(
    [str(work / "pons-emit-go"), "emit"],
    input=json.dumps(offset_request),
    text=True,
    capture_output=True,
)
assert offset_result.returncode == 0, offset_result.stderr
offset_response = json.loads(offset_result.stdout)
assert offset_response.get("error") is None, offset_response
offset_coverage = offset_response["coverage"]
assert offset_coverage["lossy"] == ["operations.events.list"], offset_coverage
assert "operations.events.list" not in offset_coverage["exact"], offset_coverage
assert offset_coverage["opaque"] == [] and offset_coverage["unsupported"] == [], offset_coverage
offset_operations = next(file["content"] for file in offset_response["files"] if file["path"] == "operations.go")
assert "EventsListAll(" not in offset_operations, offset_operations

offset_request["mode"] = "strict"
strict_result = subprocess.run(
    [str(work / "pons-emit-go"), "emit"],
    input=json.dumps(offset_request),
    text=True,
    capture_output=True,
)
assert strict_result.returncode == 0, strict_result.stderr
strict_response = json.loads(strict_result.stdout)
assert strict_response.get("error"), strict_response
assert "offset pagination" in strict_response["error"], strict_response
PY

printf 'module example.com/fixture\n\ngo 1.22\n' > "$work/module/go.mod"

cat > "$work/module/client_test.go" <<'GO'
package fixture

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestUsersGet(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/users/42" {
			t.Errorf("path = %q", request.URL.Path)
		}
		if got := request.URL.Query().Get("limit"); got != "5" {
			t.Errorf("limit = %q", got)
		}
		if got := request.URL.Query().Get("status"); got != "active" {
			t.Errorf("status = %q", got)
		}
		if got := request.Header.Get("Authorization"); got != "Bearer token" {
			t.Errorf("authorization = %q", got)
		}
		if got := request.Header.Get("X-Request-ID"); got != "req-1" {
			t.Errorf("request id = %q", got)
		}
		response.Header().Set("Content-Type", "application/json")
		response.Write([]byte(`{"id":7,"score":18446744073709551615,"handle":"simao","email":null,"status":"active","tags":["one"],"labels":{"env":"test"}}`))
	}))
	defer server.Close()

	client := &Client{BaseURL: server.URL, BearerToken: "token", APIKey: "tenant"}
	limit := int32(5)
	status := StatusActive
	requestID := "req-1"
	user, err := client.UsersGet(context.Background(), "42", UserQuery{Limit: &limit, Status: &status}, &requestID)
	if err != nil {
		t.Fatalf("UsersGet: %v", err)
	}
	if user.ID != 7 || user.Score != 18446744073709551615 {
		t.Fatalf("decoded user = %#v", user)
	}
	if user.Handle != "simao" || user.Status != StatusActive || len(user.Tags) != 1 || user.Labels["env"] != "test" {
		t.Fatalf("decoded user = %#v", user)
	}
	if user.Email != nil || user.Nickname != nil {
		t.Fatalf("nullable and optional fields = %#v", user)
	}
}

func TestTypedError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		response.Header().Set("Content-Type", "application/json")
		response.WriteHeader(http.StatusNotFound)
		response.Write([]byte(`{"code":"missing","message":"no user"}`))
	}))
	defer server.Close()

	client := &Client{BaseURL: server.URL, BearerToken: "token"}
	_, err := client.UsersGet(context.Background(), "404", UserQuery{}, nil)
	var notFound *NotFoundError
	if !errors.As(err, &notFound) {
		t.Fatalf("error = %v", err)
	}
	if notFound.StatusCode != http.StatusNotFound || notFound.Body == nil || notFound.Body.Code != "missing" {
		t.Fatalf("typed error = %#v", notFound)
	}
}

func TestUsersCreateAndList(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		switch request.Method {
		case http.MethodPost:
			if got := request.Header.Get("Authorization"); got != "Bearer token" {
				t.Errorf("authorization = %q", got)
			}
			var body CreateUser
			if err := json.NewDecoder(request.Body).Decode(&body); err != nil {
				t.Errorf("decode body: %v", err)
			}
			if body.Name != "Ada" || body.Handle != "ada" || body.Status != StatusActive {
				t.Errorf("body = %#v", body)
			}
			if body.Nickname != nil || body.Tags != nil {
				t.Errorf("optional body fields = %#v", body)
			}
			response.Header().Set("Content-Type", "application/json")
			response.WriteHeader(http.StatusCreated)
			response.Write([]byte(`{"id":8,"score":8,"handle":"ada","email":"ada@example.test","status":"active","tags":[],"labels":{}}`))
		case http.MethodGet:
			if got := request.Header.Get("X-Tenant"); got != "tenant" {
				t.Errorf("tenant = %q", got)
			}
			response.Header().Set("Content-Type", "application/json")
			response.Write([]byte(`[{"id":9,"score":9,"handle":"list","email":null,"status":"suspended","tags":[],"labels":{}}]`))
		default:
			t.Errorf("method = %s", request.Method)
		}
	}))
	defer server.Close()

	client := &Client{BaseURL: server.URL, BearerToken: "token", APIKey: "tenant"}
	user, err := client.UsersCreate(context.Background(), CreateUser{Name: "Ada", Handle: "ada", Status: StatusActive})
	if err != nil {
		t.Fatalf("UsersCreate: %v", err)
	}
	if user.ID != 8 || user.Email == nil || *user.Email != "ada@example.test" {
		t.Fatalf("created user = %#v", user)
	}
	users, err := client.UsersList(context.Background())
	if err != nil {
		t.Fatalf("UsersList: %v", err)
	}
	if len(users) != 1 || users[0].ID != 9 || users[0].Status != StatusSuspended {
		t.Fatalf("users = %#v", users)
	}
}

func TestEventsListAll(t *testing.T) {
	calls := 0
	cursors := []string{}
	limits := []string{}
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		calls++
		values := request.URL.Query()
		if _, ok := values["cursor"]; ok {
			cursors = append(cursors, values.Get("cursor"))
		} else {
			cursors = append(cursors, "<absent>")
		}
		if _, ok := values["limit"]; ok {
			limits = append(limits, values.Get("limit"))
		} else {
			limits = append(limits, "<absent>")
		}
		response.Header().Set("Content-Type", "application/json")
		switch calls {
		case 1:
			response.Write([]byte(`{"items":[{"id":1,"name":"one","details":{"source":"api","origin":"sensor","readings":[{"at":"t1","value":1.5}],"counters":{"ok":{"count":2}},"code":"c1","message":"first"}},{"id":2,"name":"two"}],"next_cursor":"c2"}`))
		case 2:
			response.Write([]byte(`{"items":[{"id":3,"name":"three"}],"next_cursor":""}`))
		default:
			t.Errorf("unexpected call %d", calls)
		}
	}))
	defer server.Close()

	client := &Client{BaseURL: server.URL}
	ids := []int64{}
	err := client.EventsListAll(context.Background(), EventQuery{}, func(event Event) error {
		ids = append(ids, event.ID)
		if event.ID == 1 {
			if event.Details == nil {
				t.Fatal("event 1 has no details")
			}
			if event.Details.Source != "api" || event.Details.Source2 == nil || *event.Details.Source2 != "sensor" {
				t.Errorf("details = %#v", event.Details)
			}
			if len(event.Details.Readings) != 1 || event.Details.Readings[0].At != "t1" || event.Details.Readings[0].Value != 1.5 {
				t.Errorf("readings = %#v", event.Details.Readings)
			}
			if got := event.Details.Counters["ok"].Count; got != 2 {
				t.Errorf("counter = %d", got)
			}
			if event.Details.Code != "c1" || event.Details.Message != "first" {
				t.Errorf("flattened fields = %#v", event.Details)
			}
		}
		if event.ID == 2 && event.Details != nil {
			t.Errorf("event 2 details = %#v", event.Details)
		}
		return nil
	})
	if err != nil {
		t.Fatalf("EventsListAll: %v", err)
	}
	if calls != 2 {
		t.Fatalf("calls = %d", calls)
	}
	if len(ids) != 3 || ids[0] != 1 || ids[1] != 2 || ids[2] != 3 {
		t.Fatalf("ids = %v", ids)
	}
	if len(cursors) != 2 || cursors[0] != "<absent>" || cursors[1] != "c2" {
		t.Fatalf("cursors = %v", cursors)
	}
	if len(limits) != 2 || limits[0] != "2" || limits[1] != "<absent>" {
		t.Fatalf("limits = %v", limits)
	}
}

func TestEventsListAllStopsOnYieldError(t *testing.T) {
	calls := 0
	yields := 0
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		calls++
		response.Header().Set("Content-Type", "application/json")
		response.Write([]byte(`{"items":[{"id":1,"name":"one"},{"id":2,"name":"two"}],"next_cursor":"c2"}`))
	}))
	defer server.Close()

	sentinel := errors.New("stop iteration")
	client := &Client{BaseURL: server.URL}
	err := client.EventsListAll(context.Background(), EventQuery{}, func(event Event) error {
		yields++
		return sentinel
	})
	if err != sentinel {
		t.Fatalf("error = %v", err)
	}
	if calls != 1 || yields != 1 {
		t.Fatalf("calls = %d, yields = %d", calls, yields)
	}
}

func TestEventsListAllReturnsPageError(t *testing.T) {
	yields := 0
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		http.Error(response, "unavailable", http.StatusInternalServerError)
	}))
	defer server.Close()

	client := &Client{BaseURL: server.URL}
	err := client.EventsListAll(context.Background(), EventQuery{}, func(event Event) error {
		yields++
		return nil
	})
	var apiError *APIError
	if !errors.As(err, &apiError) || apiError.StatusCode != http.StatusInternalServerError {
		t.Fatalf("error = %v", err)
	}
	if yields != 0 {
		t.Fatalf("yields = %d", yields)
	}
}
GO

(
  cd "$work/module"
  go test ./...
  go vet ./...
)

echo "go emitter ok"
