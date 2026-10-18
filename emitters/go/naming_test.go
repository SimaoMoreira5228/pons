package main

import "testing"

func TestExportedNamesUseGoInitialisms(t *testing.T) {
	for _, testCase := range []struct {
		words []string
		want  string
	}{
		{[]string{"user", "id"}, "UserID"},
		{[]string{"http", "api", "url"}, "HTTPAPIURL"},
		{[]string{"json", "request"}, "JSONRequest"},
		{[]string{"3d", "model"}, "N3dModel"},
		{[]string{"not", "found", "error"}, "NotFoundError"},
	} {
		if got := exportedName(testCase.words); got != testCase.want {
			t.Errorf("exportedName(%v) = %q, want %q", testCase.words, got, testCase.want)
		}
	}
}

func TestUnexportedNamesEscapeGoWords(t *testing.T) {
	for _, testCase := range []struct {
		words []string
		want  string
	}{
		{[]string{"request", "id"}, "requestID"},
		{[]string{"type"}, "typeArg"},
		{[]string{"string"}, "stringArg"},
		{[]string{"url"}, "url"},
	} {
		if got := unexportedName(testCase.words); got != testCase.want {
			t.Errorf("unexportedName(%v) = %q, want %q", testCase.words, got, testCase.want)
		}
	}
}

func TestNameRegistryClaimsWithoutReusing(t *testing.T) {
	registry := newNameRegistry()
	if got := registry.claim("User"); got != "User" {
		t.Fatalf("first claim = %q", got)
	}
	if got := registry.claim("User"); got != "User2" {
		t.Fatalf("second claim = %q", got)
	}
	if got := registry.claim("User"); got != "User3" {
		t.Fatalf("third claim = %q", got)
	}
}

func TestOperationPathEscapesWireSeparators(t *testing.T) {
	got := operationPath([][]string{{"auth", "user"}, {"log_in"}})
	want := "auth_user.log%5Fin"
	if got != want {
		t.Fatalf("operationPath = %q, want %q", got, want)
	}
}
