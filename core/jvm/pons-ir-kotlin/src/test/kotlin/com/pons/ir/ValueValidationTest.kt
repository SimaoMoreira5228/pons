package com.pons.ir

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class ValueValidationTest {
    private val document: Document = PonsIrJson.instance.decodeFromString(
        Document.serializer(),
        """
        {
          "ir_version": "0.2",
          "info": {"title": "Validation", "version": "1"},
          "types": {
            "Filter#input": {
              "name": ["filter"],
              "direction": "input",
              "kind": "struct",
              "openness": "reject",
              "fields": [
                {"wire": "slug", "ident": ["slug"], "presence": "required",
                 "ty": {"kind": "string", "pattern": "^[a-z]+$", "min_length": 2, "max_length": 8}},
                {"wire": "limit", "ident": ["limit"], "presence": "optional",
                 "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number", "minimum": 1, "maximum": 10, "multiple_of": 3}},
                {"wire": "tags", "ident": ["tags"], "presence": "optional",
                 "ty": {"kind": "array", "unique": true, "item": {"kind": "string", "pattern": "^[a-z]+$"}}},
                {"wire": "flag", "ident": ["flag"], "presence": "optional", "ty": {"kind": "bool"}},
                {"wire": "ratio", "ident": ["ratio"], "presence": "optional", "ty": {"kind": "float", "bits": 32}},
                {"wire": "amount", "ident": ["amount"], "presence": "optional",
                 "ty": {"kind": "decimal", "wire": "string"}},
                {"wire": "digest", "ident": ["digest"], "presence": "optional",
                 "ty": {"kind": "bytes", "encoding": "hex"}},
                {"wire": "payload", "ident": ["payload"], "presence": "optional", "ty": {"kind": "any"}},
                {"wire": "nothing", "ident": ["nothing"], "presence": "optional", "ty": {"kind": "null"}}
              ]
            },
            "Wrapped#input": {
              "name": ["wrapped"], "direction": "input", "kind": "newtype",
              "inner": {"ref": "Filter#input"}
            },
            "Pair#input": {
              "name": ["pair"], "direction": "input", "kind": "newtype",
              "inner": {"kind": "tuple", "items": [{"kind": "string"}, {"kind": "int", "bits": 32, "signed": true, "wire": "number"}]}
            },
            "Labels#input": {
              "name": ["labels"], "direction": "input", "kind": "newtype",
              "inner": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "string", "min_length": 2}}
            },
            "Colour#input": {
              "name": ["colour"], "direction": "input", "kind": "enum",
              "repr": {"kind": "string"},
              "variants": [
                {"wire": "red", "ident": ["red"], "payload": {"kind": "unit"}},
                {"wire": "blue", "ident": ["blue"], "payload": {"kind": "unit"}}
              ]
            },
            "Shape#input": {
              "name": ["shape"], "direction": "input", "kind": "enum",
              "repr": {"kind": "external"},
              "variants": [
                {"wire": "circle", "ident": ["circle"], "payload": {"kind": "unit"}},
                {"wire": "square", "ident": ["square"], "payload": {"kind": "struct", "fields": [
                  {"wire": "side", "ident": ["side"], "presence": "required",
                   "ty": {"kind": "int", "bits": 32, "signed": true, "wire": "number"}}
                ]}}
              ]
            },
            "Tagged#input": {
              "name": ["tagged"], "direction": "input", "kind": "enum",
              "repr": {"kind": "internal", "tag": "kind"},
              "variants": [
                {"wire": "plain", "ident": ["plain"], "payload": {"kind": "unit"}},
                {"wire": "point", "ident": ["point"], "payload": {"kind": "struct", "fields": [
                  {"wire": "x", "ident": ["x"], "presence": "required",
                   "ty": {"kind": "int", "bits": 32, "signed": true, "wire": "number"}}
                ]}}
              ]
            },
            "Adjacent#input": {
              "name": ["adjacent"], "direction": "input", "kind": "enum",
              "repr": {"kind": "adjacent", "tag": "t", "content": "c"},
              "variants": [
                {"wire": "none", "ident": ["none"], "payload": {"kind": "unit"}},
                {"wire": "value", "ident": ["value"], "payload": {"kind": "newtype", "ty": {"kind": "string"}}}
              ]
            },
            "Loose#input": {
              "name": ["loose"], "direction": "input", "kind": "enum",
              "repr": {"kind": "untagged"},
              "variants": [
                {"wire": "text", "ident": ["text"], "payload": {"kind": "newtype", "ty": {"kind": "string"}}},
                {"wire": "count", "ident": ["count"], "payload": {"kind": "newtype", "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}}}
              ]
            },
            "Node#input": {
              "name": ["node"], "direction": "input", "kind": "struct", "openness": "reject",
              "fields": [
                {"wire": "next", "ident": ["next"], "presence": "optional",
                 "ty": {"kind": "nullable", "inner": {"ref": "Node#input"}}}
              ]
            },
            "Cycle#input": {
              "name": ["cycle"], "direction": "input", "kind": "newtype",
              "inner": {"ref": "Cycle#input"}
            },
            "Open#input": {
              "name": ["open"], "direction": "input", "kind": "struct", "openness": "preserve_opaque",
              "fields": [
                {"wire": "known", "ident": ["known"], "presence": "required", "ty": {"kind": "string"}}
              ]
            },
            "Opaque#input": {
              "name": ["opaque"], "direction": "input", "kind": "newtype",
              "inner": {"kind": "opaque", "codec": "money", "wire": "text"}
            }
          }
        }
        """.trimIndent(),
    )

    private fun named(id: String): TypeRef = TypeRef.Named(NamedRef(id))

    private fun value(text: String): JsonElement = Json.parseToJsonElement(text)

    private fun reasons(ty: TypeRef, text: String): List<String> =
        validateValue(document, ty, Direction.Input, value(text)).map {
            if (it.path.isEmpty()) it.reason else "${it.path}: ${it.reason}"
        }

    @Test
    fun theFilterQueryAcceptsTheValuesItsConstraintsAllow() {
        assertEquals(
            emptyList<String>(),
            reasons(
                named("Filter#input"),
                """{"slug":"ada","limit":3,"tags":["x","y"],"flag":true,"ratio":1.5,"amount":"1.25","digest":"0a","payload":{},"nothing":null}""",
            ),
        )
    }

    @Test
    fun eachStringConstraintNamesTheValueItRejects() {
        assertEquals(
            listOf("/slug: string does not match the pattern ^[a-z]+$"),
            reasons(named("Filter#input"), """{"slug":"Ada"}"""),
        )
        assertEquals(
            listOf("/slug: string is shorter than the 2 code points it allows"),
            reasons(named("Filter#input"), """{"slug":"a"}"""),
        )
        assertEquals(
            listOf("/tags/1: string does not match the pattern ^[a-z]+$"),
            reasons(named("Filter#input"), """{"slug":"ada","tags":["x","Y"]}"""),
        )
        assertEquals(
            listOf("/tags: array item 1 repeats item 0"),
            reasons(named("Filter#input"), """{"slug":"ada","tags":["x","x"]}"""),
        )
    }

    @Test
    fun eachIntegerConstraintNamesTheValueItRejects() {
        assertEquals(
            listOf("/limit: integer is above its maximum of 10"),
            reasons(named("Filter#input"), """{"slug":"ada","limit":99}"""),
        )
        assertEquals(
            listOf("/limit: integer is below its minimum of 1"),
            reasons(named("Filter#input"), """{"slug":"ada","limit":0}"""),
        )
        assertEquals(
            listOf("/limit: integer is not a multiple of 3"),
            reasons(named("Filter#input"), """{"slug":"ada","limit":4}"""),
        )
        assertEquals(
            listOf("/limit: expected an exact integer, found 1.5"),
            reasons(named("Filter#input"), """{"slug":"ada","limit":1.5}"""),
        )
        assertEquals(
            listOf("/limit: expected a number, found a string"),
            reasons(named("Filter#input"), """{"slug":"ada","limit":"3"}"""),
        )
    }

    @Test
    fun aStructRejectsMembersItDoesNotDeclareAndMissingRequiredMembers() {
        assertEquals(
            listOf("/page: unknown property page"),
            reasons(named("Filter#input"), """{"slug":"ada","page":1}"""),
        )
        assertEquals(
            listOf("/slug: missing required field slug"),
            reasons(named("Filter#input"), """{}"""),
        )
        assertEquals(
            emptyList<String>(),
            reasons(named("Open#input"), """{"known":"x","extra":1}"""),
        )
    }

    @Test
    fun aNewtypeCarriesTheValueOfTheTypeItWraps() {
        assertEquals(
            listOf("/limit: integer is above its maximum of 10"),
            reasons(named("Wrapped#input"), """{"slug":"ada","limit":99}"""),
        )
    }

    @Test
    fun tuplesAndMapsUseThePathsOfTheirMembers() {
        assertEquals(
            listOf("expected a tuple of 2 items, found 3"),
            reasons(named("Pair#input"), """["a",1,2]"""),
        )
        assertEquals(
            listOf("/1: expected a number, found a string"),
            reasons(named("Pair#input"), """["a","b"]"""),
        )
        assertEquals(
            listOf("/key: string is shorter than the 2 code points it allows"),
            reasons(named("Labels#input"), """{"key":"a"}"""),
        )
    }

    @Test
    fun anEnumReadsEveryRepresentationTheContractPublishes() {
        assertEquals(emptyList<String>(), reasons(named("Colour#input"), "\"red\""))
        assertEquals(
            listOf("unknown enum variant green"),
            reasons(named("Colour#input"), "\"green\""),
        )
        assertEquals(emptyList<String>(), reasons(named("Shape#input"), """{"square":{"side":2}}"""))
        assertEquals(
            listOf("/square/side: expected a number, found a string"),
            reasons(named("Shape#input"), """{"square":{"side":"two"}}"""),
        )
        assertEquals(emptyList<String>(), reasons(named("Tagged#input"), """{"kind":"point","x":1}"""))
        assertEquals(
            listOf("/kind: unknown enum variant line"),
            reasons(named("Tagged#input"), """{"kind":"line","x":1}"""),
        )
        assertEquals(emptyList<String>(), reasons(named("Adjacent#input"), """{"t":"value","c":"x"}"""))
        assertEquals(
            listOf("/c: missing required field c"),
            reasons(named("Adjacent#input"), """{"t":"value"}"""),
        )
        assertEquals(emptyList<String>(), reasons(named("Loose#input"), "\"text\""))
        assertEquals(emptyList<String>(), reasons(named("Loose#input"), "3"))
        assertEquals(
            listOf("value matches none of the enum variants"),
            reasons(named("Loose#input"), "true"),
        )
    }

    @Test
    fun aRecursiveStructReadsTheNestedValueWhileANewtypeCycleIsReported() {
        assertEquals(
            emptyList<String>(),
            reasons(named("Node#input"), """{"next":{"next":null}}"""),
        )
        assertEquals(
            listOf("type Cycle#input cannot be resolved because newtypes form a cycle"),
            reasons(named("Cycle#input"), "\"x\""),
        )
    }

    @Test
    fun anOpaqueValueIsRejectedBecauseItsSchemaIsUnknown() {
        val violations = validateValue(
            document,
            named("Opaque#input"),
            Direction.Input,
            JsonPrimitive("unreadable"),
        )
        assertEquals(1, violations.size)
        assertTrue(violations.single().reason.contains("opaque"))
        assertTrue(violations.single().reason.contains("money"))
    }

    @Test
    fun aReferenceToAnUnpublishedTypeIsReportedWhereItIsUsed() {
        assertEquals(
            listOf("unknown type missing#input"),
            reasons(named("missing#input"), "1"),
        )
    }
}
