package main

import (
	"fmt"
	"strings"
)

type paginationPlan struct {
	AllName      string
	ItemType     string
	Items        []string
	ItemsPointer bool
	Next         *paginationField
	Cursor       paginationField
	PageSize     *paginationSizePlan
	Locals       paginationLocals
}

type paginationField struct {
	Carrier  string
	Selector []string
	GoType   string
	BaseType string
	Pointer  bool
	Kind     string
}

type paginationSizePlan struct {
	paginationField
	Default uint32
}

type paginationLocals struct {
	page      string
	callErr   string
	cursor    string
	next      string
	first     string
	pageSize  string
	nextValue string
	item      string
	yieldErr  string
	query     string
	body      string
	bodyBase  string
}

type resolvedField struct {
	Ty       typeRef
	GoType   string
	BaseType string
	Pointer  bool
	Selector []string
}

type stepResult struct {
	Ty     typeRef
	GoType string
	Field  string
}

func (a *analysis) buildPagination(plan *operationPlan, origin string) *paginationPlan {
	descriptor := plan.Op.Semantics.Pagination
	if descriptor == nil {
		return nil
	}
	subject := "operations." + origin
	if descriptor.Request.Kind == "offset" {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because offset pagination is not supported", subject)
		return nil
	}
	if descriptor.Request.Kind != "cursor" {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the request kind %q is not supported", subject, descriptor.Request.Kind)
		return nil
	}
	if plan.Success == nil || plan.Success.Type == nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the operation has no JSON success body", subject)
		return nil
	}
	cursor, err := a.paginationField(plan, descriptor.Request.Field)
	if err != nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the cursor field %q %s", subject, descriptor.Request.Field, err)
		return nil
	}
	items, err := a.resolvePointer(*plan.Success.Type, plan.Success.GoType, descriptor.Items)
	if err != nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the items pointer %q does not resolve: %s", subject, descriptor.Items, err)
		return nil
	}
	terminal, err := a.unwrapTerminal(items.Ty, map[string]bool{})
	if err != nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the items pointer %q does not resolve: %s", subject, descriptor.Items, err)
		return nil
	}
	if terminal.Inline == nil || terminal.Inline.Kind != "array" || terminal.Inline.Item == nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the items pointer %q does not end at an array", subject, descriptor.Items)
		return nil
	}
	itemType, err := a.goType(*terminal.Inline.Item)
	if err != nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the items pointer %q has an item type that %s", subject, descriptor.Items, err)
		return nil
	}
	if descriptor.Next == nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because cursor pagination needs a next pointer", subject)
		return nil
	}
	next, err := a.resolvePointer(*plan.Success.Type, plan.Success.GoType, *descriptor.Next)
	if err != nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the next pointer %q does not resolve: %s", subject, *descriptor.Next, err)
		return nil
	}
	nextTerminal, err := a.unwrapTerminal(next.Ty, map[string]bool{})
	if err != nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the next pointer %q does not resolve: %s", subject, *descriptor.Next, err)
		return nil
	}
	nextKind, err := a.cursorKind(nextTerminal, map[string]bool{})
	if err != nil {
		a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the next pointer %q %s", subject, *descriptor.Next, err)
		return nil
	}
	var size *paginationSizePlan
	if descriptor.PageSize != nil && descriptor.PageSize.Default != nil {
		if *descriptor.PageSize.Default == 0 {
			a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the page size default is zero", subject)
			return nil
		}
		field, err := a.paginationField(plan, descriptor.PageSize.Field)
		if err != nil {
			a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the page size field %q %s", subject, descriptor.PageSize.Field, err)
			return nil
		}
		if field.Kind != "int" {
			a.markLossy(origin, "the Go emitter cannot generate a pagination iterator for %s, because the page size field %q is not an integer", subject, descriptor.PageSize.Field)
			return nil
		}
		size = &paginationSizePlan{paginationField: field, Default: *descriptor.PageSize.Default}
	}
	return &paginationPlan{
		ItemType:     itemType,
		Items:        items.Selector,
		ItemsPointer: items.Pointer,
		Next: &paginationField{
			Carrier:  "page",
			Selector: next.Selector,
			GoType:   next.GoType,
			BaseType: next.BaseType,
			Pointer:  next.Pointer,
			Kind:     nextKind,
		},
		Cursor:   cursor,
		PageSize: size,
		Locals:   paginationLocalsFor(plan),
	}
}

func (a *analysis) nameIterator(plan *operationPlan) {
	if plan.Pagination == nil {
		return
	}
	origin := operationPath(plan.Op.ID)
	allName := plan.Name + "All"
	if owner, taken := a.methodOwners[allName]; taken {
		a.addProblem(origin, "the iterator method %s for operations.%s collides with the generated member %s", allName, origin, owner)
		return
	}
	a.methodNames.reserve(allName)
	a.methodOwners[allName] = "operations." + origin
	plan.Pagination.AllName = allName
}

func paginationLocalsFor(plan *operationPlan) paginationLocals {
	registry := newNameRegistry()
	for _, param := range plan.Params {
		registry.reserve(param.Name)
	}
	registry.reserve("ctx")
	registry.reserve("yield")
	return paginationLocals{
		page:      registry.claim("page"),
		callErr:   registry.claim("err"),
		cursor:    registry.claim("cursor"),
		next:      registry.claim("next"),
		first:     registry.claim("first"),
		pageSize:  registry.claim("pageSize"),
		nextValue: registry.claim("nextValue"),
		item:      registry.claim("item"),
		yieldErr:  registry.claim("yieldErr"),
		query:     registry.claim("pageQuery"),
		body:      registry.claim("pageBody"),
		bodyBase:  registry.claim("bodyBase"),
	}
}

func (a *analysis) paginationField(plan *operationPlan, wire string) (paginationField, error) {
	var candidates []paginationField
	if plan.QueryParam != "" && plan.Op.Query != nil {
		if step, err := a.step(*plan.Op.Query, wire); err == nil {
			field, err := a.paginationValue("query", step)
			if err != nil {
				return paginationField{}, err
			}
			candidates = append(candidates, field)
		}
	}
	if plan.Body != nil {
		if step, err := a.step(plan.Body.Ty, wire); err == nil {
			field, err := a.paginationValue("body", step)
			if err != nil {
				return paginationField{}, err
			}
			candidates = append(candidates, field)
		}
	}
	switch len(candidates) {
	case 0:
		return paginationField{}, fmt.Errorf("is not declared on the query or body")
	case 1:
		return candidates[0], nil
	default:
		return paginationField{}, fmt.Errorf("is declared on both the query and the body")
	}
}

func (a *analysis) paginationValue(carrier string, step stepResult) (paginationField, error) {
	base, pointer := splitGoPointer(step.GoType)
	kind, err := a.cursorKind(step.Ty, map[string]bool{})
	if err != nil {
		return paginationField{}, err
	}
	return paginationField{
		Carrier:  carrier,
		Selector: []string{step.Field},
		GoType:   step.GoType,
		BaseType: base,
		Pointer:  pointer,
		Kind:     kind,
	}, nil
}

func (a *analysis) resolvePointer(ref typeRef, goType, pointer string) (resolvedField, error) {
	segments, err := pointerSegments(pointer)
	if err != nil {
		return resolvedField{}, err
	}
	result := resolvedField{Ty: ref, GoType: goType}
	for _, segment := range segments {
		step, err := a.step(result.Ty, segment)
		if err != nil {
			return resolvedField{}, err
		}
		result.Ty = step.Ty
		result.GoType = step.GoType
		if step.Field != "" {
			result.Selector = append(result.Selector, step.Field)
		}
	}
	result.BaseType, result.Pointer = splitGoPointer(result.GoType)
	return result, nil
}

func (a *analysis) step(ref typeRef, segment string) (stepResult, error) {
	if ref.Named {
		if ref.Variant != "" {
			return stepResult{}, fmt.Errorf("reaches an enum variant at the segment %q", segment)
		}
		definition, ok := a.doc.Types[ref.ID]
		if !ok {
			return stepResult{}, fmt.Errorf("names the unknown type %s", ref.ID)
		}
		switch definition.Kind {
		case "newtype":
			if definition.Inner == nil {
				return stepResult{}, fmt.Errorf("has a newtype without an inner type")
			}
			return a.step(*definition.Inner, segment)
		case "struct":
			if plan := a.typeByID[ref.ID]; plan != nil {
				for _, field := range plan.Fields {
					if field.Wire == segment {
						return stepResult{Ty: field.Ty, GoType: field.GoType, Field: field.Name}, nil
					}
				}
			}
			for _, flattened := range definition.Flattened {
				if result, err := a.step(flattened, segment); err == nil {
					return result, nil
				}
			}
			return stepResult{}, fmt.Errorf("does not find the field %q", segment)
		case "enum":
			return stepResult{}, fmt.Errorf("reaches an enum at the segment %q", segment)
		default:
			return stepResult{}, fmt.Errorf("has the unsupported kind %q", definition.Kind)
		}
	}
	if ref.Inline == nil {
		return stepResult{}, fmt.Errorf("is not a type reference")
	}
	switch ref.Inline.Kind {
	case "nullable":
		if ref.Inline.Inner == nil {
			return stepResult{}, fmt.Errorf("has a nullable wrapper with no inner type")
		}
		return a.step(*ref.Inline.Inner, segment)
	case "array":
		return stepResult{}, fmt.Errorf("reaches an array at the segment %q", segment)
	case "map":
		return stepResult{}, fmt.Errorf("reaches a map at the segment %q", segment)
	case "any":
		return stepResult{}, fmt.Errorf("reaches any at the segment %q", segment)
	case "opaque":
		return stepResult{}, fmt.Errorf("reaches an opaque value at the segment %q", segment)
	default:
		return stepResult{}, fmt.Errorf("ends before the segment %q", segment)
	}
}

func (a *analysis) unwrapTerminal(ref typeRef, seen map[string]bool) (typeRef, error) {
	if ref.Named {
		if ref.Variant != "" {
			return ref, fmt.Errorf("reaches an enum variant")
		}
		definition, ok := a.doc.Types[ref.ID]
		if !ok {
			return ref, fmt.Errorf("names the unknown type %s", ref.ID)
		}
		switch definition.Kind {
		case "newtype":
			if seen[ref.ID] {
				return ref, fmt.Errorf("forms a newtype cycle")
			}
			seen[ref.ID] = true
			if definition.Inner == nil {
				return ref, fmt.Errorf("has a newtype without an inner type")
			}
			return a.unwrapTerminal(*definition.Inner, seen)
		case "enum":
			return ref, fmt.Errorf("reaches an enum")
		default:
			return ref, nil
		}
	}
	if ref.Inline != nil && ref.Inline.Kind == "nullable" {
		if ref.Inline.Inner == nil {
			return ref, fmt.Errorf("has a nullable wrapper with no inner type")
		}
		return a.unwrapTerminal(*ref.Inline.Inner, seen)
	}
	return ref, nil
}

func (a *analysis) cursorKind(ref typeRef, seen map[string]bool) (string, error) {
	if ref.Named {
		if ref.Variant != "" {
			return "", fmt.Errorf("is an enum variant")
		}
		definition, ok := a.doc.Types[ref.ID]
		if !ok {
			return "", fmt.Errorf("names the unknown type %s", ref.ID)
		}
		switch definition.Kind {
		case "newtype":
			if seen[ref.ID] {
				return "", fmt.Errorf("forms a newtype cycle")
			}
			seen[ref.ID] = true
			if definition.Inner == nil {
				return "", fmt.Errorf("has a newtype without an inner type")
			}
			return a.cursorKind(*definition.Inner, seen)
		case "enum":
			return "", fmt.Errorf("is an enum")
		default:
			return "", fmt.Errorf("is not a string or integer")
		}
	}
	if ref.Inline == nil {
		return "", fmt.Errorf("is not a type reference")
	}
	switch ref.Inline.Kind {
	case "nullable":
		if ref.Inline.Inner == nil {
			return "", fmt.Errorf("has a nullable wrapper with no inner type")
		}
		return a.cursorKind(*ref.Inline.Inner, seen)
	case "string":
		return "string", nil
	case "int":
		return "int", nil
	default:
		return "", fmt.Errorf("is not a string or integer")
	}
}

func pointerSegments(pointer string) ([]string, error) {
	if pointer == "" {
		return nil, nil
	}
	rest, ok := strings.CutPrefix(pointer, "/")
	if !ok {
		return nil, fmt.Errorf("is not a JSON pointer")
	}
	segments := strings.Split(rest, "/")
	decoded := make([]string, 0, len(segments))
	for _, segment := range segments {
		value, err := decodePointerSegment(segment)
		if err != nil {
			return nil, err
		}
		decoded = append(decoded, value)
	}
	return decoded, nil
}

func decodePointerSegment(segment string) (string, error) {
	var builder strings.Builder
	for index := 0; index < len(segment); index++ {
		if segment[index] != '~' {
			builder.WriteByte(segment[index])
			continue
		}
		if index+1 >= len(segment) {
			return "", fmt.Errorf("has an incomplete JSON pointer escape")
		}
		index++
		switch segment[index] {
		case '0':
			builder.WriteByte('~')
		case '1':
			builder.WriteByte('/')
		default:
			return "", fmt.Errorf("has an invalid JSON pointer escape")
		}
	}
	return builder.String(), nil
}

func splitGoPointer(goType string) (string, bool) {
	base := strings.TrimPrefix(goType, "*")
	return base, base != goType
}
