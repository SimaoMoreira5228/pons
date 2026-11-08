package main

import (
	"fmt"
	"strings"
)

func renderDoc(a *analysis) string {
	var builder strings.Builder
	builder.WriteString(generatedMarker)
	fmt.Fprintf(&builder, "\n// Package %s is a Go client for %s.\npackage %s\n", a.pkg, commentText(a.doc.Info.Title), a.pkg)
	return builder.String()
}

func renderTypes(a *analysis) string {
	var builder strings.Builder
	var imports []string
	if a.typesUseRawJSON() {
		imports = append(imports, "encoding/json")
	}
	if len(a.queryTypes) > 0 {
		imports = append(imports, "net/url")
	}
	writeFileHeader(&builder, a.pkg, imports)

	for _, plan := range a.types {
		builder.WriteString("\n")
		switch plan.Kind {
		case "struct":
			fmt.Fprintf(&builder, "type %s struct {\n", plan.Name)
			writeStructMembers(&builder, plan.Fields)
			builder.WriteString("}\n")
		case "enum":
			fmt.Fprintf(&builder, "type %s string\n", plan.Name)
			if len(plan.Consts) == 0 {
				continue
			}
			builder.WriteString("\nconst (\n")
			for _, constant := range plan.Consts {
				fmt.Fprintf(&builder, "\t%s %s = %q\n", constant.Name, plan.Name, constant.Wire)
			}
			builder.WriteString(")\n")
		case "newtype":
			fmt.Fprintf(&builder, "type %s %s\n", plan.Name, plan.Inner)
		}
	}

	queryIDs := sortedKeys(a.queryTypes)
	for _, id := range queryIDs {
		plan := a.typeByID[id]
		if plan == nil || plan.Kind != "struct" {
			continue
		}
		builder.WriteString("\n")
		fmt.Fprintf(&builder, "func (v %s) queryValues() url.Values {\n", plan.Name)
		builder.WriteString("\tvalues := url.Values{}\n")
		for _, field := range plan.Fields {
			if field.Optional {
				fmt.Fprintf(&builder, "\tif v.%s != nil {\n", field.Name)
				fmt.Fprintf(&builder, "\t\tvalues.Set(%q, scalarText(*v.%s))\n", field.Wire, field.Name)
				builder.WriteString("\t}\n")
				continue
			}
			fmt.Fprintf(&builder, "\tvalues.Set(%q, scalarText(v.%s))\n", field.Wire, field.Name)
		}
		builder.WriteString("\treturn values\n")
		builder.WriteString("}\n")
	}

	return builder.String()
}

func writeStructMembers(builder *strings.Builder, fields []fieldPlan) {
	for _, field := range fields {
		fmt.Fprintf(builder, "\t%s %s `%s`\n", field.Name, field.GoType, field.Tag)
	}
}

func anonymousStruct(fields []fieldPlan) string {
	var builder strings.Builder
	builder.WriteString("struct {\n")
	writeStructMembers(&builder, fields)
	builder.WriteString("}")
	return builder.String()
}
