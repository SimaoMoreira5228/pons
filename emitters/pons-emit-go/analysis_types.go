package main

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
)

func (a *analysis) buildTypePlans() {
	ids := sortedKeys(a.doc.Types)
	for _, id := range ids {
		def := a.doc.Types[id]
		plan := &typePlan{
			ID:   id,
			Name: a.typeNames[id],
			Kind: def.Kind,
			Def:  def,
		}
		switch def.Kind {
		case "struct":
			a.buildStructPlan(plan)
		case "enum":
			a.buildEnumPlan(plan)
		case "newtype":
			if def.Inner == nil {
				a.addProblem(id, "the newtype %s has no inner type", id)
				continue
			}
			inner, err := a.goType(*def.Inner, "types."+id)
			if err != nil {
				a.addProblem(id, "%s", err)
				continue
			}
			plan.Inner = inner
		default:
			a.addProblem(id, "the type kind %q is not supported", def.Kind)
			continue
		}
		a.types = append(a.types, plan)
		a.typeByID[id] = plan
	}
}
func (a *analysis) buildStructPlan(plan *typePlan) {
	def := plan.Def
	if len(def.Flattened) > 0 {
		a.addProblem(plan.ID, "the struct %s uses flattened members, which this emitter does not support", plan.ID)
	}
	subject := "types." + plan.ID
	a.classifyOpenness(def.Openness, subject, "the struct "+plan.ID)
	plan.Fields = a.buildFields(def.Fields, newNameRegistry(), map[string]bool{}, subject, "the struct "+plan.ID)
}
func (a *analysis) buildFields(fields []field, names *nameRegistry, wires map[string]bool, subject, label string) []fieldPlan {
	plans := []fieldPlan{}
	for _, field := range fields {
		if wires[field.Wire] {
			a.addProblem(originOf(subject), "%s has more than one field with the wire name %q", label, field.Wire)
			continue
		}
		if field.Access != nil && *field.Access != "read_write" {
			a.addProblem(originOf(subject), "%s field %s is %s; this emitter only generates read_write fields", label, field.Wire, *field.Access)
			continue
		}
		optional := field.Presence == "optional"
		if optional && isNullable(field.Ty) {
			a.addProblem(originOf(subject), "%s field %s is optional and nullable; Go needs a pointer to distinguish absent from null, which this emitter does not generate", label, field.Wire)
			continue
		}
		goType, err := a.goType(field.Ty, subject)
		if err != nil {
			a.addProblem(originOf(subject), "%s field %s %s", label, field.Wire, err)
			continue
		}
		if err := validateJSONTag(field.Wire); err != nil {
			a.addProblem(originOf(subject), "%s field %s %s", label, field.Wire, err)
			continue
		}
		wires[field.Wire] = true
		if optional {
			goType = "*" + goType
		}
		words := field.Ident
		if len(words) == 0 {
			words = wordsFromWire(field.Wire)
		}
		plans = append(plans, fieldPlan{
			Wire:     field.Wire,
			Name:     names.claim(exportedName(words)),
			GoType:   goType,
			Tag:      quoteTag(field.Wire, optional),
			Ty:       field.Ty,
			Optional: optional,
		})
	}
	return plans
}
func (a *analysis) structFields(inline *inlineType, subject string) []fieldPlan {
	const label = "an inline struct"
	a.classifyOpenness(inline.Openness, subject, label)
	names := newNameRegistry()
	wires := map[string]bool{}
	plans := a.buildFields(inline.Fields, names, wires, subject, label)
	for _, flattened := range inline.Flattened {
		plans = append(plans, a.flattenedFields(flattened, names, wires, subject, label, map[string]bool{})...)
	}
	return plans
}
func (a *analysis) flattenedFields(ref typeRef, names *nameRegistry, wires map[string]bool, subject, label string, seen map[string]bool) []fieldPlan {
	if ref.Named {
		if ref.Variant != "" {
			a.addProblem(originOf(subject), "%s flattens the %s variant of %s, which cannot be inlined", label, ref.Variant, ref.ID)
			return nil
		}
		if seen[ref.ID] {
			a.addProblem(originOf(subject), "%s flattens %s in a cycle", label, ref.ID)
			return nil
		}
		def, ok := a.doc.Types[ref.ID]
		if !ok {
			a.addProblem(originOf(subject), "%s flattens the unknown type %s", label, ref.ID)
			return nil
		}
		switch def.Kind {
		case "struct":
			seen[ref.ID] = true
			a.classifyOpenness(def.Openness, subject, "the flattened struct "+ref.ID)
			plans := a.buildFields(def.Fields, names, wires, subject, label)
			for _, nested := range def.Flattened {
				plans = append(plans, a.flattenedFields(nested, names, wires, subject, label, seen)...)
			}
			delete(seen, ref.ID)
			return plans
		case "newtype":
			if def.Inner == nil {
				a.addProblem(originOf(subject), "%s flattens the newtype %s, which has no inner type", label, ref.ID)
				return nil
			}
			seen[ref.ID] = true
			plans := a.flattenedFields(*def.Inner, names, wires, subject, label, seen)
			delete(seen, ref.ID)
			return plans
		default:
			a.addProblem(originOf(subject), "%s flattens the %s %s; only struct members can be inlined into one object", label, def.Kind, ref.ID)
			return nil
		}
	}
	if ref.Inline == nil {
		a.addProblem(originOf(subject), "%s flattens a value that is not a type reference", label)
		return nil
	}
	if ref.Inline.Kind != "struct" {
		a.addProblem(originOf(subject), "%s flattens an inline %s; only struct members can be inlined into one object", label, ref.Inline.Kind)
		return nil
	}
	a.classifyOpenness(ref.Inline.Openness, subject, label)
	plans := a.buildFields(ref.Inline.Fields, names, wires, subject, label)
	for _, nested := range ref.Inline.Flattened {
		plans = append(plans, a.flattenedFields(nested, names, wires, subject, label, seen)...)
	}
	return plans
}

func (a *analysis) buildEnumPlan(plan *typePlan) {
	def := plan.Def
	if def.Repr == nil {
		a.addProblem(plan.ID, "the enum %s has no representation", plan.ID)
		return
	}
	if def.Repr.Kind != "string" {
		a.addProblem(plan.ID, "the enum %s uses the %s representation; this emitter only generates string enums", plan.ID, def.Repr.Kind)
		return
	}
	constNames := newNameRegistry()
	for _, variant := range def.Variants {
		if variant.Payload.Kind != "unit" {
			a.addProblem(plan.ID, "the string enum %s has a %s payload on variant %s; only unit variants are supported", plan.ID, variant.Payload.Kind, variant.Wire)
			continue
		}
		words := variant.Ident
		if len(words) == 0 {
			words = wordsFromWire(variant.Wire)
		}
		constName := constNames.claim(plan.Name + exportedName(words))
		plan.Consts = append(plan.Consts, constPlan{Name: constName, Wire: variant.Wire})
	}
}
func (a *analysis) goType(ref typeRef, subject string) (string, error) {
	if ref.Named {
		name, ok := a.typeNames[ref.ID]
		if !ok {
			return "", fmt.Errorf("references the unknown type %s", ref.ID)
		}
		return name, nil
	}
	if ref.Inline == nil {
		return "", fmt.Errorf("is not a type reference")
	}
	inline := ref.Inline
	switch inline.Kind {
	case "bool":
		return "bool", nil
	case "int":
		if inline.Wire != "number" {
			return "", fmt.Errorf("uses an integer on the %q wire; this emitter writes JSON numbers only", inline.Wire)
		}
		switch inline.Bits {
		case 8:
			if inline.Signed {
				return "int8", nil
			}
			return "uint8", nil
		case 16:
			if inline.Signed {
				return "int16", nil
			}
			return "uint16", nil
		case 32:
			if inline.Signed {
				return "int32", nil
			}
			return "uint32", nil
		case 64:
			if inline.Signed {
				return "int64", nil
			}
			return "uint64", nil
		default:
			return "", fmt.Errorf("uses a %d-bit integer; this emitter supports up to 64 bits", inline.Bits)
		}
	case "float":
		switch inline.Bits {
		case 32:
			return "float32", nil
		case 64:
			return "float64", nil
		default:
			return "", fmt.Errorf("uses a %d-bit float", inline.Bits)
		}
	case "decimal":
		if inline.Wire == "string" {
			return "string", nil
		}
		return "", fmt.Errorf("uses a decimal on the %q wire; this emitter preserves decimals as strings", inline.Wire)
	case "string":
		return "string", nil
	case "bytes":
		if inline.Encoding == "base64" {
			return "[]byte", nil
		}
		return "", fmt.Errorf("uses %q encoded bytes; this emitter supports base64 bytes", inline.Encoding)
	case "null":
		return "", fmt.Errorf("uses the null type, which has no Go value")
	case "any":
		return "json.RawMessage", nil
	case "opaque":
		return "", fmt.Errorf("uses the opaque codec %q, which this emitter does not implement", inline.Codec)
	case "nullable":
		if inline.Inner == nil {
			return "", fmt.Errorf("has a nullable wrapper with no inner type")
		}
		inner, err := a.goType(*inline.Inner, subject)
		if err != nil {
			return "", err
		}
		if strings.HasPrefix(inner, "*") {
			return "", fmt.Errorf("nests nullable wrappers, which this emitter does not support")
		}
		return "*" + inner, nil
	case "array":
		if inline.Item == nil {
			return "", fmt.Errorf("has an array with no item type")
		}
		item, err := a.goType(*inline.Item, subject)
		if err != nil {
			return "", err
		}
		return "[]" + item, nil
	case "map":
		if inline.Key == nil || inline.Value == nil {
			return "", fmt.Errorf("has a map without a key or value type")
		}
		if !a.stringUnderlying(*inline.Key) {
			return "", fmt.Errorf("uses a map key that is not string-like")
		}
		key, err := a.goType(*inline.Key, subject)
		if err != nil {
			return "", err
		}
		value, err := a.goType(*inline.Value, subject)
		if err != nil {
			return "", err
		}
		return "map[" + key + "]" + value, nil
	case "tuple":
		return "", fmt.Errorf("uses a tuple, which this emitter does not generate")
	case "struct":
		return anonymousStruct(a.structFields(inline, subject)), nil
	default:
		return "", fmt.Errorf("uses the unsupported type kind %q", inline.Kind)
	}
}
func (a *analysis) stringUnderlying(ref typeRef) bool {
	if !ref.Named {
		return ref.Inline != nil && ref.Inline.Kind == "string"
	}
	def, ok := a.doc.Types[ref.ID]
	if !ok {
		return false
	}
	switch def.Kind {
	case "enum":
		return def.Repr != nil && def.Repr.Kind == "string"
	case "newtype":
		return def.Inner != nil && a.stringUnderlying(*def.Inner)
	default:
		return false
	}
}
func (a *analysis) scalar(ref typeRef, seen map[string]bool) bool {
	if !ref.Named {
		if ref.Inline == nil {
			return false
		}
		switch ref.Inline.Kind {
		case "bool", "int", "float", "string", "decimal":
			return true
		default:
			return false
		}
	}
	if seen[ref.ID] {
		return false
	}
	seen[ref.ID] = true
	def, ok := a.doc.Types[ref.ID]
	if !ok {
		return false
	}
	switch def.Kind {
	case "enum":
		return def.Repr != nil && def.Repr.Kind == "string"
	case "newtype":
		return def.Inner != nil && a.scalar(*def.Inner, seen)
	default:
		return false
	}
}

type opennessPolicy int

const (
	closedObject opennessPolicy = iota
	typedOpenObject
	opaqueOpenObject
)

func (a *analysis) classifyOpenness(raw json.RawMessage, subject, label string) {
	policy, err := decodeOpenness(raw)
	if err != nil {
		a.addProblem(originOf(subject), "%s %s", label, err)
		return
	}
	switch policy {
	case typedOpenObject:
		a.markLossySubject(subject, "%s preserves typed unknown properties, which this emitter drops when decoding; the object openness is lossy", label)
	case opaqueOpenObject:
		a.markLossySubject(subject, "%s preserves unknown properties, which this emitter drops when decoding; the object openness is lossy", label)
	}
}

func decodeOpenness(raw json.RawMessage) (opennessPolicy, error) {
	var word string
	if err := json.Unmarshal(raw, &word); err == nil {
		switch word {
		case "reject":
			return closedObject, nil
		case "preserve_opaque":
			return opaqueOpenObject, nil
		default:
			return closedObject, fmt.Errorf("uses object openness %q, which this emitter does not know", word)
		}
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(raw, &object); err == nil {
		if _, ok := object["preserve_typed"]; ok {
			return typedOpenObject, nil
		}
	}
	return closedObject, fmt.Errorf("has no usable object openness policy")
}
func validateJSONTag(wire string) error {
	if strings.ContainsAny(wire, "`,") {
		return fmt.Errorf("has a wire name that cannot be written as a Go JSON tag")
	}
	return nil
}
func quoteTag(wire string, optional bool) string {
	if optional {
		return fmt.Sprintf("json:%q", wire+",omitempty")
	}
	return fmt.Sprintf("json:%q", wire)
}
func isNullable(ref typeRef) bool {
	return !ref.Named && ref.Inline != nil && ref.Inline.Kind == "nullable"
}

func (a *analysis) checkValueCycles() {
	edges := map[string][]string{}
	for id, def := range a.doc.Types {
		switch def.Kind {
		case "struct":
			for _, field := range def.Fields {
				if field.Presence == "optional" || isNullable(field.Ty) {
					continue
				}
				edges[id] = append(edges[id], valueEdges(field.Ty)...)
			}
		case "newtype":
			if def.Inner != nil {
				edges[id] = append(edges[id], valueEdges(*def.Inner)...)
			}
		}
	}
	state := map[string]int{}
	reported := map[string]bool{}
	stack := []string{}
	var visit func(string)
	visit = func(id string) {
		state[id] = 1
		stack = append(stack, id)
		for _, next := range edges[id] {
			switch state[next] {
			case 0:
				visit(next)
			case 1:
				start := 0
				for index, member := range stack {
					if member == next {
						start = index
						break
					}
				}
				cycle := append([]string{}, stack[start:]...)
				sort.Strings(cycle)
				key := strings.Join(cycle, " -> ")
				if !reported[key] {
					reported[key] = true
					a.addProblem(id, "the types %s form a required cycle; make one field optional or nullable so Go can hold the value", key)
				}
			}
		}
		stack = stack[:len(stack)-1]
		state[id] = 2
	}
	for _, id := range sortedKeys(a.doc.Types) {
		if state[id] == 0 {
			visit(id)
		}
	}
}

func valueEdges(ref typeRef) []string {
	if ref.Named {
		return []string{ref.ID}
	}
	if ref.Inline == nil || ref.Inline.Kind != "struct" {
		return nil
	}
	var edges []string
	for _, field := range ref.Inline.Fields {
		if field.Presence == "optional" || isNullable(field.Ty) {
			continue
		}
		edges = append(edges, valueEdges(field.Ty)...)
	}
	for _, flattened := range ref.Inline.Flattened {
		edges = append(edges, valueEdges(flattened)...)
	}
	return edges
}
