package main

import (
	"fmt"
	"net/url"
	"strings"
)

func (a *analysis) buildOperationPlans() {
	for index := range a.doc.Operations {
		op := &a.doc.Operations[index]
		plan := &operationPlan{Op: op}
		origin := operationPath(op.ID)
		if !validMethod(op.Method) {
			a.addProblem(origin, "the operation uses the unsupported HTTP method %q", op.Method)
		}
		if rawPresent(op.Stream) {
			a.addProblem(origin, "the operation streams responses, which this emitter does not support")
		}
		if rawPresent(op.Provides) {
			a.addProblem(origin, "the operation provides a credential, which this emitter does not support")
		}
		if len(op.Auth.Scopes) > 0 {
			a.addProblem(origin, "the operation declares auth scopes, which this emitter does not support")
		}
		params := newNameRegistry()
		for _, reserved := range []string{"ctx", "requestURL", "err", "request", "httpResponse", "responseBody", "payload", "out", "yield"} {
			params.reserve(reserved)
		}
		a.buildPath(plan, params, origin)
		a.buildQuery(plan, params, origin)
		a.buildHeaders(plan, params, origin)
		a.buildBody(plan, params, origin)
		a.buildSuccess(plan, origin)
		plan.Pagination = a.buildPagination(plan, origin)
		a.buildErrors(plan, origin)
		plan.Auth = a.observeAuth(op, origin)
		plan.Name = a.methodNames.claim(exportedName(joinWords(op.ID)))
		a.methodOwners[plan.Name] = "operations." + origin
		a.operations = append(a.operations, plan)
	}
	for _, plan := range a.operations {
		a.nameIterator(plan)
	}
}
func (a *analysis) buildPath(plan *operationPlan, params *nameRegistry, origin string) {
	for _, segment := range plan.Op.Path {
		switch {
		case segment.Lit != nil:
			plan.Path = append(plan.Path, pathPiece{Literal: url.PathEscape(*segment.Lit)})
		case segment.Param != nil && segment.Ty != nil:
			if !a.scalar(*segment.Ty, map[string]bool{}) || isNullable(*segment.Ty) {
				a.addProblem(origin, "the path parameter %s is not a scalar", *segment.Param)
				continue
			}
			goType, err := a.goType(*segment.Ty, "operations."+origin)
			if err != nil {
				a.addProblem(origin, "the path parameter %s %s", *segment.Param, err)
				continue
			}
			name := params.claim(unexportedName(wordsFromWire(*segment.Param)))
			plan.Params = append(plan.Params, paramPlan{Kind: "path", Name: name, GoType: goType})
			plan.Path = append(plan.Path, pathPiece{Param: name})
			a.usesPathEscape = true
		default:
			a.addProblem(origin, "a path segment has neither a literal nor a parameter with a type")
		}
	}
}
func (a *analysis) buildQuery(plan *operationPlan, params *nameRegistry, origin string) {
	query := plan.Op.Query
	if query == nil {
		return
	}
	if !query.Named {
		a.addProblem(origin, "the query is not a named struct, which this emitter requires")
		return
	}
	queryPlan, ok := a.typeByID[query.ID]
	if !ok || queryPlan.Kind != "struct" {
		a.addProblem(origin, "the query type %s is not a struct", query.ID)
		return
	}
	for _, field := range queryPlan.Def.Fields {
		if !a.scalar(field.Ty, map[string]bool{}) || isNullable(field.Ty) {
			a.addProblem(origin, "the query field %s.%s is not a scalar", query.ID, field.Wire)
		}
	}
	a.queryTypes[query.ID] = true
	plan.QueryType = queryPlan.Name
	plan.QueryParam = params.claim("query")
	plan.Params = append(plan.Params, paramPlan{Kind: "query", Name: plan.QueryParam, GoType: queryPlan.Name})
}
func (a *analysis) buildHeaders(plan *operationPlan, params *nameRegistry, origin string) {
	for _, header := range plan.Op.Headers {
		optional := header.Presence == "optional"
		if optional && isNullable(header.Ty) {
			a.addProblem(origin, "the request header %s is optional and nullable, which this emitter does not support", header.Wire)
			continue
		}
		if !a.scalar(header.Ty, map[string]bool{}) {
			a.addProblem(origin, "the request header %s is not a scalar", header.Wire)
			continue
		}
		goType, err := a.goType(header.Ty, "operations."+origin)
		if err != nil {
			a.addProblem(origin, "the request header %s %s", header.Wire, err)
			continue
		}
		if optional {
			goType = "*" + goType
		}
		words := header.Ident
		if len(words) == 0 {
			words = wordsFromWire(header.Wire)
		}
		name := params.claim(unexportedName(words))
		plan.Params = append(plan.Params, paramPlan{
			Kind:     "header",
			Name:     name,
			GoType:   goType,
			Wire:     header.Wire,
			Optional: optional,
		})
	}
}
func (a *analysis) buildBody(plan *operationPlan, params *nameRegistry, origin string) {
	requestBody := plan.Op.Body
	if requestBody == nil {
		return
	}
	if requestBody.Content != "json" {
		a.addProblem(origin, "the request body content %q is not supported; this emitter reads JSON bodies", requestBody.Content)
		return
	}
	if err := checkJSONMediaType(requestBody.Media); err != nil {
		a.addProblem(origin, "the request body %s", err)
		return
	}
	if !requestBody.Required && isNullable(requestBody.Ty) {
		a.addProblem(origin, "the optional request body is nullable, which this emitter does not support")
		return
	}
	goType, err := a.goType(requestBody.Ty, "operations."+origin)
	if err != nil {
		a.addProblem(origin, "the request body %s", err)
		return
	}
	name := params.claim("body")
	if requestBody.Required {
		plan.Body = &bodyPlan{Param: name, GoType: goType, Ty: requestBody.Ty}
	} else {
		plan.Body = &bodyPlan{Param: name, GoType: "*" + goType, Optional: true, Ty: requestBody.Ty}
	}
	plan.Params = append(plan.Params, paramPlan{Kind: "body", Name: name, GoType: plan.Body.GoType})
	a.usesPayload = true
}
func (a *analysis) buildSuccess(plan *operationPlan, origin string) {
	if len(plan.Op.Responses) != 1 {
		a.addProblem(origin, "the operation has %d success responses; this emitter generates one", len(plan.Op.Responses))
		return
	}
	response := plan.Op.Responses[0]
	if response.Status.Code < 200 || response.Status.Code > 299 {
		a.addProblem(origin, "the success status %s is not an exact 2xx status", statusText(response.Status))
		return
	}
	if len(response.Headers) > 0 {
		a.addProblem(origin, "the success response declares typed headers, which this emitter does not expose")
		return
	}
	if err := checkJSONMediaType(response.Media); err != nil {
		a.addProblem(origin, "the success response %s", err)
		return
	}
	switch response.Content {
	case "json":
		if response.Body == nil {
			a.addProblem(origin, "the JSON success response has no body type")
			return
		}
		goType, err := a.goType(*response.Body, "operations."+origin)
		if err != nil {
			a.addProblem(origin, "the success body %s", err)
			return
		}
		a.usesJSONBody = true
		plan.Success = &successPlan{
			Status:  response.Status.Code,
			Type:    response.Body,
			GoType:  goType,
			Pointer: response.Body.Named,
		}
	case "empty":
		if response.Body != nil {
			a.addProblem(origin, "the empty success response declares a body type")
			return
		}
		plan.Success = &successPlan{Status: response.Status.Code}
	default:
		a.addProblem(origin, "the success response content %q is not supported; this emitter reads JSON or empty bodies", response.Content)
	}
}
func (a *analysis) buildErrors(plan *operationPlan, origin string) {
	errors := append([]errorResponse{}, plan.Op.Errors...)
	if plan.Op.inheritsErrors() {
		errors = append(errors, a.doc.CommonErrors...)
	}
	for index := range errors {
		key, ok := a.observeError(&errors[index], origin)
		if ok {
			plan.ErrorKeys = append(plan.ErrorKeys, key)
		}
	}
}
func (a *analysis) observeError(response *errorResponse, origin string) (string, bool) {
	if response.Status.Code < 100 || response.Status.Code > 599 {
		a.addProblem(origin, "the error status %s is not an exact status", statusText(response.Status))
		return "", false
	}
	if response.Status.Class != 0 || response.Status.Default || response.Status.Unknown {
		a.addProblem(origin, "the error status %s is not an exact status", statusText(response.Status))
		return "", false
	}
	if len(response.Headers) > 0 {
		a.addProblem(origin, "the error %s declares typed headers, which this emitter does not expose", statusText(response.Status))
		return "", false
	}
	if err := checkJSONMediaType(response.Media); err != nil {
		a.addProblem(origin, "the error %s %s", statusText(response.Status), err)
		return "", false
	}
	bodyGo := ""
	switch response.Content {
	case "json":
		if response.Body == nil {
			a.addProblem(origin, "the JSON error %s has no body type", statusText(response.Status))
			return "", false
		}
		goType, err := a.goType(*response.Body, "operations."+origin)
		if err != nil {
			a.addProblem(origin, "the error body %s", err)
			return "", false
		}
		bodyGo = goType
	case "text":
		if response.Body == nil {
			a.addProblem(origin, "the text error %s has no body type", statusText(response.Status))
			return "", false
		}
	case "empty":
		if response.Body != nil {
			a.addProblem(origin, "the empty error %s declares a body type", statusText(response.Status))
			return "", false
		}
	default:
		a.addProblem(origin, "the error content %q is not supported", response.Content)
		return "", false
	}
	key := errorKey(*response)
	if _, ok := a.errorDefs[key]; !ok {
		message := "request failed"
		if response.Doc != nil && response.Doc.Summary != nil && *response.Doc.Summary != "" {
			message = *response.Doc.Summary
		} else if len(response.Name) > 0 {
			message = strings.ToLower(strings.Join(response.Name, " "))
		}
		a.errorDefs[key] = &errorType{
			Key:     key,
			Message: message,
			Content: response.Content,
			Body:    response.Body,
			BodyGo:  bodyGo,
			Words:   response.Name,
			Status:  statusText(response.Status),
			Code:    response.Status.Code,
		}
	}
	return key, true
}
func (a *analysis) observeAuth(op *operation, origin string) authPlan {
	plan := authPlan{Required: !op.Auth.Optional && len(op.Auth.Alternatives) > 0}
	if len(op.Auth.Alternatives) > 1 {
		a.addProblem(origin, "the operation has %d auth alternatives; this emitter attaches one alternative", len(op.Auth.Alternatives))
	}
	for _, alternative := range op.Auth.Alternatives {
		for _, id := range alternative {
			scheme, ok := a.doc.AuthSchemes[id]
			if !ok {
				a.addProblem(origin, "the auth scheme %s is not declared", id)
				continue
			}
			switch scheme.Kind {
			case "bearer":
				a.bearerSchemes[id] = true
				plan.Bearer = true
			case "api_key":
				if scheme.In != "header" && scheme.In != "query" {
					a.addProblem(origin, "the api key scheme %s is carried in %q; this emitter supports headers and queries", id, scheme.In)
					continue
				}
				if scheme.Name == "" {
					a.addProblem(origin, "the api key scheme %s has no carrier name", id)
					continue
				}
				a.apiKeySchemes[id] = true
				if scheme.In == "header" {
					a.apiKeyHeader = scheme.Name
				} else {
					a.apiKeyQuery = scheme.Name
				}
				plan.APIKey = true
			default:
				a.addProblem(origin, "the auth scheme %s uses the %s kind, which this emitter does not support", id, scheme.Kind)
			}
		}
	}
	return plan
}
func checkJSONMediaType(media *mediaDescriptor) error {
	if media == nil {
		return nil
	}
	if len(media.Parameters) > 0 {
		return fmt.Errorf("uses media type parameters, which this emitter does not write")
	}
	if media.Codec != nil {
		return fmt.Errorf("declares media codec %q, which this emitter does not implement", *media.Codec)
	}
	if strings.ToLower(media.Type) != "application" || strings.ToLower(media.Subtype) != "json" {
		return fmt.Errorf("uses the media type %s/%s; this emitter writes application/json", media.Type, media.Subtype)
	}
	return nil
}
func refSignature(ref typeRef) string {
	if ref.Named {
		return "ref:" + ref.ID + "#" + ref.Variant
	}
	if ref.Inline == nil {
		return "invalid"
	}
	inline := ref.Inline
	switch inline.Kind {
	case "nullable", "array":
		if inline.Inner != nil {
			return inline.Kind + "(" + refSignature(*inline.Inner) + ")"
		}
		if inline.Item != nil {
			return inline.Kind + "(" + refSignature(*inline.Item) + ")"
		}
	case "map":
		if inline.Key != nil && inline.Value != nil {
			return "map(" + refSignature(*inline.Key) + "," + refSignature(*inline.Value) + ")"
		}
	case "tuple":
		parts := make([]string, 0, len(inline.Items))
		for _, item := range inline.Items {
			parts = append(parts, refSignature(item))
		}
		return "tuple(" + strings.Join(parts, ",") + ")"
	case "struct":
		parts := make([]string, 0, len(inline.Fields)+len(inline.Flattened)+1)
		for _, field := range inline.Fields {
			parts = append(parts, field.Wire+"="+refSignature(field.Ty))
		}
		for _, flat := range inline.Flattened {
			parts = append(parts, "flat("+refSignature(flat)+")")
		}
		parts = append(parts, "open("+string(inline.Openness)+")")
		return "struct(" + strings.Join(parts, ",") + ")"
	}
	return inline.Kind
}
func validMethod(method string) bool {
	switch method {
	case "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS":
		return true
	default:
		return false
	}
}
