package main

import (
	"fmt"
	"strconv"
	"strings"
)

func renderOperations(a *analysis) string {
	var builder strings.Builder
	if len(a.operations) == 0 {
		writeFileHeader(&builder, a.pkg, nil)
		return builder.String()
	}
	imports := []string{"context", "io", "net/http"}
	if a.usesJSONBody || a.usesPayload || a.usesErrorJSON() {
		imports = append(imports, "encoding/json")
	}
	if a.usesPayload {
		imports = append(imports, "bytes")
	}
	if a.usesPathEscape {
		imports = append(imports, "net/url")
	}
	writeFileHeader(&builder, a.pkg, imports)

	for _, plan := range a.operations {
		renderOperation(&builder, a, plan)
	}
	for _, plan := range a.operations {
		if plan.Pagination != nil {
			renderOperationAll(&builder, plan)
		}
	}
	return builder.String()
}

func renderOperation(builder *strings.Builder, a *analysis, plan *operationPlan) {
	builder.WriteString("\n")
	fmt.Fprintf(builder, "func (c *Client) %s(%s) %s {\n", plan.Name, parameterList(plan), operationReturn(plan))
	queryArgument := "nil"
	if plan.QueryParam != "" {
		queryArgument = plan.QueryParam + ".queryValues()"
	}
	fmt.Fprintf(builder, "\trequestURL, err := c.endpoint(%s, %s)\n", pathExpression(plan.Path), queryArgument)
	fmt.Fprintf(builder, "\tif err != nil {\n\t\t%s\n\t}\n", failureReturn(plan))

	reader := "nil"
	if plan.Body != nil {
		if plan.Body.Optional {
			builder.WriteString("\tvar payload []byte\n")
			fmt.Fprintf(builder, "\tif %s != nil {\n", plan.Body.Param)
			fmt.Fprintf(builder, "\t\tpayload, err = json.Marshal(%s)\n", plan.Body.Param)
			fmt.Fprintf(builder, "\t\tif err != nil {\n\t\t\t%s\n\t\t}\n\t}\n", failureReturn(plan))
		} else {
			fmt.Fprintf(builder, "\tpayload, err := json.Marshal(%s)\n", plan.Body.Param)
			fmt.Fprintf(builder, "\tif err != nil {\n\t\t%s\n\t}\n", failureReturn(plan))
		}
		reader = "bytes.NewReader(payload)"
	}
	fmt.Fprintf(builder, "\trequest, err := http.NewRequestWithContext(ctx, %q, requestURL, %s)\n", plan.Op.Method, reader)
	fmt.Fprintf(builder, "\tif err != nil {\n\t\t%s\n\t}\n", failureReturn(plan))

	if plan.Success != nil && plan.Success.Type != nil {
		builder.WriteString("\trequest.Header.Set(\"Accept\", \"application/json\")\n")
	}
	if plan.Body != nil {
		builder.WriteString("\trequest.Header.Set(\"Content-Type\", \"application/json\")\n")
	}
	for _, param := range plan.Params {
		if param.Kind != "header" {
			continue
		}
		if param.Optional {
			fmt.Fprintf(builder, "\tif %s != nil {\n", param.Name)
			fmt.Fprintf(builder, "\t\trequest.Header.Set(%q, scalarText(*%s))\n", param.Wire, param.Name)
			builder.WriteString("\t}\n")
			continue
		}
		fmt.Fprintf(builder, "\trequest.Header.Set(%q, scalarText(%s))\n", param.Wire, param.Name)
	}
	if plan.Auth.Bearer || plan.Auth.APIKey {
		fmt.Fprintf(builder, "\tif err := c.authorize(request, %t, %t, %t); err != nil {\n\t\t%s\n\t}\n",
			plan.Auth.Bearer, plan.Auth.APIKey, plan.Auth.Required, failureReturn(plan))
	}
	builder.WriteString("\thttpResponse, err := c.httpClient().Do(request)\n")
	fmt.Fprintf(builder, "\tif err != nil {\n\t\t%s\n\t}\n", failureReturn(plan))
	builder.WriteString("\tdefer httpResponse.Body.Close()\n")
	builder.WriteString("\tresponseBody, err := io.ReadAll(httpResponse.Body)\n")
	fmt.Fprintf(builder, "\tif err != nil {\n\t\t%s\n\t}\n", failureReturn(plan))

	builder.WriteString("\tswitch httpResponse.StatusCode {\n")
	if plan.Success != nil {
		renderSuccessCase(builder, plan)
	}
	for _, key := range plan.ErrorKeys {
		renderErrorCase(builder, plan, a.errorDefs[key])
	}
	fmt.Fprintf(builder, "\tdefault:\n\t\treturn %s&APIError{StatusCode: httpResponse.StatusCode, Status: httpResponse.Status, Header: httpResponse.Header, Body: responseBody}\n\t}\n}\n", nilPrefix(plan))
}

func renderSuccessCase(builder *strings.Builder, plan *operationPlan) {
	success := plan.Success
	fmt.Fprintf(builder, "\tcase %d:\n", success.Status)
	if success.Type == nil {
		builder.WriteString("\t\treturn nil\n")
		return
	}
	fmt.Fprintf(builder, "\t\tvar out %s\n", success.GoType)
	fmt.Fprintf(builder, "\t\tif err := json.Unmarshal(responseBody, &out); err != nil {\n\t\t\treturn %s&DecodeError{StatusCode: httpResponse.StatusCode, Body: responseBody, Err: err}\n\t\t}\n", nilPrefix(plan))
	if success.Pointer {
		builder.WriteString("\t\treturn &out, nil\n")
	} else {
		builder.WriteString("\t\treturn out, nil\n")
	}
}

func renderErrorCase(builder *strings.Builder, plan *operationPlan, def *errorType) {
	if def == nil {
		return
	}
	fmt.Fprintf(builder, "\tcase %d:\n", def.Code)
	fmt.Fprintf(builder, "\t\tvar typed %s\n", def.Name)
	builder.WriteString("\t\ttyped.StatusCode = httpResponse.StatusCode\n")
	builder.WriteString("\t\ttyped.Header = httpResponse.Header\n")
	switch def.Content {
	case "json":
		fmt.Fprintf(builder, "\t\tif err := json.Unmarshal(responseBody, &typed.Body); err != nil {\n\t\t\treturn %s&DecodeError{StatusCode: httpResponse.StatusCode, Body: responseBody, Err: err}\n\t\t}\n", nilPrefix(plan))
	case "text":
		builder.WriteString("\t\ttyped.Body = string(responseBody)\n")
	}
	fmt.Fprintf(builder, "\t\treturn %s&typed\n", nilPrefix(plan))
}

func parameterList(plan *operationPlan) string {
	parts := []string{"ctx context.Context"}
	for _, param := range plan.Params {
		parts = append(parts, param.Name+" "+param.GoType)
	}
	return strings.Join(parts, ", ")
}

func operationReturn(plan *operationPlan) string {
	if plan.Success == nil || plan.Success.Type == nil {
		return "error"
	}
	if plan.Success.Pointer {
		return "(*" + plan.Success.GoType + ", error)"
	}
	return "(" + plan.Success.GoType + ", error)"
}

func nilPrefix(plan *operationPlan) string {
	if plan.Success == nil || plan.Success.Type == nil {
		return ""
	}
	return "nil, "
}

func failureReturn(plan *operationPlan) string {
	return "return " + nilPrefix(plan) + "err"
}

func pathExpression(pieces []pathPiece) string {
	parts := make([]string, 0, len(pieces))
	var literal strings.Builder
	flush := func() {
		if literal.Len() == 0 {
			return
		}
		parts = append(parts, strconv.Quote(literal.String()))
		literal.Reset()
	}
	for _, piece := range pieces {
		if piece.Param == "" {
			literal.WriteString("/")
			literal.WriteString(piece.Literal)
			continue
		}
		literal.WriteString("/")
		flush()
		parts = append(parts, "url.PathEscape(scalarText("+piece.Param+"))")
	}
	flush()
	if len(parts) == 0 {
		return `"/"`
	}
	return strings.Join(parts, " + ")
}
