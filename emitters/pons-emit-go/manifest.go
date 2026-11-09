package main

type manifest struct {
	Name         string       `json:"name"`
	Version      string       `json:"version"`
	Protocol     versionRange `json:"protocol"`
	IRVersion    versionRange `json:"ir_version"`
	Capabilities capabilities `json:"capabilities"`
	OptionSchema optionSchema `json:"option_schema"`
}

type versionRange struct {
	Min string `json:"min"`
	Max string `json:"max"`
}

type capabilities struct {
	Streaming      []string          `json:"streaming"`
	Pagination     []string          `json:"pagination"`
	EnumReprs      []string          `json:"enum_reprs"`
	DirectionSplit bool              `json:"direction_split"`
	Provides       bool              `json:"provides"`
	CustomAuth     bool              `json:"custom_auth"`
	Flattened      bool              `json:"flattened"`
	WideIntegers   bool              `json:"wide_integers"`
	Opaque         bool              `json:"opaque"`
	ObjectOpenness []string          `json:"object_openness"`
	Validation     validationSupport `json:"validation"`
}

type validationSupport struct {
	Backends     []string          `json:"backends"`
	Capabilities map[string]string `json:"capabilities"`
}

type optionSchema struct {
	Options map[string]optionSpec `json:"options"`
}

type optionSpec struct {
	Type     string `json:"type"`
	Required bool   `json:"required"`
}

func buildManifest() manifest {
	return manifest{
		Name:      "go",
		Version:   "0.1.0",
		Protocol:  versionRange{Min: "3.0", Max: "3.0"},
		IRVersion: versionRange{Min: "0.3", Max: "0.3"},
		Capabilities: capabilities{
			Streaming:      []string{},
			Pagination:     []string{"pagination.cursor"},
			EnumReprs:      []string{"string"},
			DirectionSplit: true,
			Provides:       false,
			CustomAuth:     false,
			Flattened:      false,
			WideIntegers:   true,
			Opaque:         false,
			ObjectOpenness: []string{"reject"},
			Validation: validationSupport{
				Backends:     []string{},
				Capabilities: map[string]string{},
			},
		},
		OptionSchema: optionSchema{
			Options: map[string]optionSpec{
				"package": {Type: "string", Required: false},
			},
		},
	}
}
