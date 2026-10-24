package com.pons.ir

import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Test

class CoverageTest {
    @Test
    fun coverageKeepsEveryLevelAndSortsSubjects() {
        val coverage = Coverage(
            exact = linkedSetOf("types.User", "operations.auth.login"),
            lossy = setOf("types.Lossy"),
        )
        val encoded = PonsIrJson.instance.encodeToString(Coverage.serializer(), coverage)
        val decoded = PonsIrJson.instance.decodeFromString(Coverage.serializer(), encoded)
        assertEquals(coverage, decoded)

        val element = PonsIrJson.instance.parseToJsonElement(encoded).jsonObject
        assertEquals(
            listOf("operations.auth.login", "types.User"),
            element.getValue("exact").jsonArray.map { it.toString().trim('"') },
        )
        assertEquals(0, element.getValue("opaque").jsonArray.size)
    }

    @Test
    fun coverageRejectsDuplicateSubjectsAndUnknownLevels() {
        for (level in listOf("exact", "opaque", "lossy", "unsupported")) {
            val encoded = when (level) {
                "exact" -> """{"exact":["types.User","types.User"],"opaque":[],"lossy":[],"unsupported":[]}"""
                "opaque" -> """{"exact":[],"opaque":["types.User","types.User"],"lossy":[],"unsupported":[]}"""
                "lossy" -> """{"exact":[],"opaque":[],"lossy":["types.User","types.User"],"unsupported":[]}"""
                else -> """{"exact":[],"opaque":[],"lossy":[],"unsupported":["types.User","types.User"]}"""
            }
            assertThrows(SerializationException::class.java) {
                PonsIrJson.instance.decodeFromString(Coverage.serializer(), encoded)
            }
        }

        assertThrows(SerializationException::class.java) {
            PonsIrJson.instance.decodeFromString(
                ValidationCoverage.serializer(),
                """{"exact":[],"lossy":["types.User","types.User"],"unsupported":[]}""",
            )
        }

        assertThrows(SerializationException::class.java) {
            PonsIrJson.instance.decodeFromString(
                ValidationCoverage.serializer(),
                """{"exact":[],"opaque":[],"lossy":[],"unsupported":[]}""",
            )
        }
    }
}
