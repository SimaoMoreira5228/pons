package com.pons.ir

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

private val inlineStructDocument = """
    {
      "ir_version": "0.3",
      "info": {"title": "Inline structs", "version": "1"},
      "types": {
        "Address#input": {
          "name": ["address"],
          "direction": "input",
          "kind": "struct",
          "openness": "reject",
          "fields": [
            {"wire": "city", "ident": ["city"], "presence": "required", "ty": {"kind": "string"}}
          ]
        },
        "Order#input": {
          "name": ["order"],
          "direction": "input",
          "kind": "struct",
          "openness": "reject",
          "fields": [
            {"wire": "shipping", "ident": ["shipping"], "presence": "required",
             "ty": {"kind": "struct", "openness": "reject", "fields": [
               {"wire": "city", "ident": ["city"], "presence": "required", "ty": {"kind": "string"}}
             ]}},
            {"wire": "merged", "ident": ["merged"], "presence": "required",
             "ty": {"kind": "struct", "openness": "reject",
                    "flattened": [{"ref": "Address#input"}],
                    "fields": [
                      {"wire": "note", "ident": ["note"], "presence": "optional", "ty": {"kind": "string"}}
                    ]}},
            {"wire": "extras", "ident": ["extras"], "presence": "optional",
             "ty": {"kind": "struct", "openness": {"preserve_typed": {"value": {"ref": "Address#input"}}},
                    "fields": []}},
            {"wire": "opaque", "ident": ["opaque"], "presence": "optional",
             "ty": {"kind": "struct", "openness": "preserve_opaque",
                    "fields": [
                      {"wire": "known", "ident": ["known"], "presence": "optional", "ty": {"kind": "bool"}}
                    ]}}
          ]
        }
      }
    }
""".trimIndent()

class InlineStructTest {
    private val document: Document =
        PonsIrJson.instance.decodeFromString(Document.serializer(), inlineStructDocument)

    private fun named(id: String): TypeRef = TypeRef.Named(NamedRef(id))

    private fun value(text: String): JsonElement = Json.parseToJsonElement(text)

    private fun reasons(text: String): List<String> =
        validateValue(document, named("Order#input"), Direction.Input, value(text)).map {
            if (it.path.isEmpty()) it.reason else "${it.path}: ${it.reason}"
        }

    @Test
    fun anInlineStructSurvivesTheJsonRoundTrip() {
        val encoded = PonsIrJson.instance.encodeToString(Document.serializer(), document)

        assertEquals(
            PonsIrJson.instance.parseToJsonElement(inlineStructDocument),
            PonsIrJson.instance.parseToJsonElement(encoded),
        )
        assertEquals(
            document,
            PonsIrJson.instance.decodeFromString(Document.serializer(), encoded),
        )
    }

    @Test
    fun anInlineStructValidatesFieldsFlattenedMembersAndOpenness() {
        assertEquals(
            emptyList<String>(),
            reasons(
                """{"shipping":{"city":"Lisbon"},"merged":{"city":"Porto","note":"ring"},"extras":{"home":{"city":"Faro"}},"opaque":{"known":true,"extra":1}}""",
            ),
        )
    }

    @Test
    fun anInlineStructRejectsValuesItsNamedTwinWouldReject() {
        assertEquals(
            listOf(
                "/shipping/zip: unknown property zip",
                "/merged/city: missing required field city",
                "/extras/home/city: expected a string, found a number",
            ),
            reasons(
                """{"shipping":{"city":"Lisbon","zip":1},"merged":{"note":"ring"},"extras":{"home":{"city":2}}}""",
            ),
        )
    }
}
