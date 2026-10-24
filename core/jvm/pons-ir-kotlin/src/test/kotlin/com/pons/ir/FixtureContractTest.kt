package com.pons.ir

import com.fasterxml.jackson.databind.ObjectMapper
import com.networknt.schema.JsonSchema
import com.networknt.schema.JsonSchemaFactory
import com.networknt.schema.SpecVersion
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.io.File
import java.nio.file.Files

private val repoRoot: File =
    System.getProperty("pons.repoRoot")?.let(::File)
        ?: generateSequence(File("").absoluteFile) { it.parentFile }
            .first { File(it, "spec/fixtures").isDirectory }

private val fixtureDirectory = File(repoRoot, "spec/fixtures")

private val fixtures =
    fixtureDirectory.listFiles { file -> file.isFile && file.extension == "json" }
        ?.sortedBy { it.name }
        .orEmpty()

private val schema: JsonSchema by lazy {
    val source = Files.readString(File(repoRoot, "spec/ir.schema.json").toPath())
    JsonSchemaFactory.getInstance(SpecVersion.VersionFlag.V202012).getSchema(source)
}

private val schemaMapper = ObjectMapper()

private fun assertValidAgainstSchema(label: String, json: String) {
    val messages = schema.validate(schemaMapper.readTree(json))
    assertTrue(
        messages.isEmpty(),
        "$label does not match ir.schema.json:\n" +
            messages.joinToString("\n") { "${it.instanceLocation}: ${it.message}" },
    )
}

class FixtureContractTest {
    @Test
    fun everyFixtureRoundTrips() {
        assertTrue(fixtures.isNotEmpty(), "no JSON fixtures found in $fixtureDirectory")
        for (fixture in fixtures) {
            val source = fixture.readText()
            val document = PonsIrJson.instance.decodeFromString(Document.serializer(), source)
            val encoded = PonsIrJson.instance.encodeToString(Document.serializer(), document)
            assertEquals(
                PonsIrJson.instance.parseToJsonElement(source),
                PonsIrJson.instance.parseToJsonElement(encoded),
                "${fixture.name} changed during the round trip:\n$encoded",
            )
        }
    }

    @Test
    fun everyFixtureAndItsRoundTripMatchTheSchema() {
        for (fixture in fixtures) {
            val source = fixture.readText()
            assertValidAgainstSchema(fixture.name, source)
            val document = PonsIrJson.instance.decodeFromString(Document.serializer(), source)
            val encoded = PonsIrJson.instance.encodeToString(Document.serializer(), document)
            assertValidAgainstSchema("${fixture.name} (round trip)", encoded)
        }
    }

    @Test
    fun unknownMembersAreRejected() {
        val source = File(fixtureDirectory, "login.json").readText()
        val document = PonsIrJson.instance.decodeFromString(Document.serializer(), source)
        val encoded = PonsIrJson.instance
            .parseToJsonElement(PonsIrJson.instance.encodeToString(Document.serializer(), document))
            .jsonObject

        val documentWithUnknown = JsonObject(encoded + ("unexpected" to JsonPrimitive(true)))
        assertThrows(SerializationException::class.java) {
            PonsIrJson.instance.decodeFromString(Document.serializer(), documentWithUnknown.toString())
        }
        assertTrue(
            schema.validate(schemaMapper.readTree(documentWithUnknown.toString())).isNotEmpty(),
            "ir.schema.json accepted a document with an unknown member",
        )

        val types = encoded.getValue("types").jsonObject
        val userWithUnknown = JsonObject(
            types.getValue("User").jsonObject + ("unexpected" to JsonPrimitive(true)),
        )
        val typeWithUnknown = JsonObject(
            encoded + ("types" to JsonObject(types + ("User" to userWithUnknown))),
        )
        assertThrows(SerializationException::class.java) {
            PonsIrJson.instance.decodeFromString(Document.serializer(), typeWithUnknown.toString())
        }
    }

    @Test
    fun explicitNullFieldDefaultSurvivesTheRoundTrip() {
        val source = File(fixtureDirectory, "login.json").readText()
        val document = PonsIrJson.instance.decodeFromString(Document.serializer(), source)
        val user = document.types.getValue("User")
        val struct = user.definition as TypeKind.StructType
        val field = struct.fields.first().copy(default = FieldDefault.Value(JsonNull))
        val fields = listOf(field) + struct.fields.drop(1)
        val withNullDefault = document.copy(
            types = document.types + ("User" to user.copy(definition = struct.copy(fields = fields))),
        )

        val encoded = PonsIrJson.instance.encodeToString(Document.serializer(), withNullDefault)
        val decoded = PonsIrJson.instance.decodeFromString(Document.serializer(), encoded)
        assertEquals(withNullDefault, decoded)

        val encodedUser = PonsIrJson.instance.parseToJsonElement(encoded).jsonObject
            .getValue("types").jsonObject.getValue("User").jsonObject
        val encodedField = encodedUser.getValue("fields").jsonArray.first().jsonObject
        assertTrue("default" in encodedField)
    }
}
