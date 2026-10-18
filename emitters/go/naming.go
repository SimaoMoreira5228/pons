package main

import (
	"fmt"
	"strings"
	"unicode"
)

var goKeywords = map[string]bool{
	"break": true, "case": true, "chan": true, "const": true, "continue": true,
	"default": true, "defer": true, "else": true, "fallthrough": true, "for": true,
	"func": true, "go": true, "goto": true, "if": true, "import": true,
	"interface": true, "map": true, "package": true, "range": true, "return": true,
	"select": true, "struct": true, "switch": true, "type": true, "var": true,
}

var predeclared = map[string]bool{
	"any": true, "bool": true, "byte": true, "comparable": true, "complex64": true,
	"complex128": true, "error": true, "float32": true, "float64": true, "int": true,
	"int8": true, "int16": true, "int32": true, "int64": true, "rune": true,
	"string": true, "uint": true, "uint8": true, "uint16": true, "uint32": true,
	"uint64": true, "uintptr": true, "true": true, "false": true, "iota": true,
	"nil": true,
}

var initialisms = map[string]string{
	"api": "API", "ascii": "ASCII", "cpu": "CPU", "css": "CSS", "dns": "DNS",
	"eof": "EOF", "guid": "GUID", "html": "HTML", "http": "HTTP", "https": "HTTPS",
	"id": "ID", "ip": "IP", "json": "JSON", "qps": "QPS", "ram": "RAM",
	"rpc": "RPC", "sla": "SLA", "smtp": "SMTP", "sql": "SQL", "ssh": "SSH",
	"tcp": "TCP", "tls": "TLS", "ttl": "TTL", "udp": "UDP", "ui": "UI",
	"uid": "UID", "uri": "URI", "url": "URL", "utf8": "UTF8", "uuid": "UUID",
	"vm": "VM", "xml": "XML", "xmpp": "XMPP", "xsrf": "XSRF", "xss": "XSS",
}

type nameRegistry struct {
	used map[string]bool
}

func newNameRegistry() *nameRegistry {
	return &nameRegistry{used: map[string]bool{}}
}

func (r *nameRegistry) claim(base string) string {
	if base == "" {
		base = "Value"
	}
	if !r.used[base] {
		r.used[base] = true
		return base
	}
	for index := 2; ; index++ {
		candidate := fmt.Sprintf("%s%d", base, index)
		if !r.used[candidate] {
			r.used[candidate] = true
			return candidate
		}
	}
}

func (r *nameRegistry) reserve(name string) {
	r.used[name] = true
}

func exportedName(words []string) string {
	var builder strings.Builder
	for _, word := range words {
		builder.WriteString(titleWord(word))
	}
	name := builder.String()
	if name == "" {
		return "Value"
	}
	if !unicode.IsLetter(rune(name[0])) {
		name = "N" + name
	}
	return name
}

func unexportedName(words []string) string {
	var builder strings.Builder
	for index, word := range words {
		cleaned := sanitizeWord(word)
		if cleaned == "" {
			continue
		}
		if index == 0 {
			builder.WriteString(strings.ToLower(cleaned))
			continue
		}
		builder.WriteString(titleWord(cleaned))
	}
	name := builder.String()
	if name == "" {
		name = "value"
	}
	if !unicode.IsLetter(rune(name[0])) {
		name = "value" + name
	}
	if goKeywords[name] || predeclared[name] {
		name += "Arg"
	}
	return name
}

func titleWord(word string) string {
	cleaned := sanitizeWord(word)
	if cleaned == "" {
		return ""
	}
	lower := strings.ToLower(cleaned)
	if initialism, ok := initialisms[lower]; ok {
		return initialism
	}
	runes := []rune(lower)
	runes[0] = unicode.ToUpper(runes[0])
	return string(runes)
}

func sanitizeWord(word string) string {
	var builder strings.Builder
	for _, character := range word {
		if character < unicode.MaxASCII {
			if unicode.IsLetter(character) || unicode.IsDigit(character) {
				builder.WriteRune(character)
			}
		}
	}
	return builder.String()
}

func joinWords(words [][]string) []string {
	var joined []string
	for _, segment := range words {
		joined = append(joined, segment...)
	}
	return joined
}

func wordsFromWire(wire string) []string {
	parts := strings.FieldsFunc(wire, func(character rune) bool {
		return !unicode.IsLetter(character) && !unicode.IsDigit(character)
	})
	if len(parts) == 0 {
		return []string{"value"}
	}
	return parts
}
