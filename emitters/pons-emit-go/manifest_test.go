package main

import "testing"

func TestManifestDescribesTheSupportedSlice(t *testing.T) {
	manifest := buildManifest()
	if manifest.Name != "go" || manifest.Version != "0.1.0" {
		t.Fatalf("manifest = %#v", manifest)
	}
	if manifest.Protocol != (versionRange{Min: "3.0", Max: "3.0"}) {
		t.Fatalf("protocol = %#v", manifest.Protocol)
	}
	if manifest.IRVersion != (versionRange{Min: "0.2", Max: "0.2"}) {
		t.Fatalf("ir version = %#v", manifest.IRVersion)
	}
	if got := manifest.Capabilities.EnumReprs; len(got) != 1 || got[0] != "string" {
		t.Fatalf("enum representations = %v", got)
	}
	if got := manifest.Capabilities.ObjectOpenness; len(got) != 1 || got[0] != "reject" {
		t.Fatalf("object openness = %v", got)
	}
	if !manifest.Capabilities.WideIntegers {
		t.Fatal("64-bit integers should be declared")
	}
	if manifest.Capabilities.Provides || manifest.Capabilities.CustomAuth ||
		manifest.Capabilities.Flattened || manifest.Capabilities.Opaque {
		t.Fatalf("capabilities claim more than the emitter does: %#v", manifest.Capabilities)
	}
	if len(manifest.Capabilities.Streaming) != 0 {
		t.Fatalf("streaming = %v", manifest.Capabilities.Streaming)
	}
	spec, ok := manifest.OptionSchema.Options["package"]
	if !ok || spec.Type != "string" || spec.Required {
		t.Fatalf("package option = %#v", spec)
	}
}

func TestPackageOptionDefaultsAndRejectsBadNames(t *testing.T) {
	name, err := packageOption(nil)
	if err != nil || name != "client" {
		t.Fatalf("packageOption(nil) = %q, %v", name, err)
	}
	for _, bad := range []string{"", "_", "main", "type", "two words", "9lives"} {
		if err := validatePackageName(bad); err == nil {
			t.Errorf("validatePackageName(%q) accepted a bad name", bad)
		}
	}
	for _, good := range []string{"client", "api_v2", "fixture"} {
		if err := validatePackageName(good); err != nil {
			t.Errorf("validatePackageName(%q) = %v", good, err)
		}
	}
}
