@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.pons.ir

import kotlinx.serialization.EncodeDefault
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.descriptors.element
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder

@Serializable(with = CoverageSerializer::class)
data class Coverage(
    val exact: Set<String> = emptySet(),
    val opaque: Set<String> = emptySet(),
    val lossy: Set<String> = emptySet(),
    val unsupported: Set<String> = emptySet(),
)

@Serializable(with = ValidationCoverageSerializer::class)
data class ValidationCoverage(
    val exact: Set<String> = emptySet(),
    val lossy: Set<String> = emptySet(),
    val unsupported: Set<String> = emptySet(),
)

@Serializable
private data class CoverageWire(
    @EncodeDefault(EncodeDefault.Mode.ALWAYS) val exact: List<String> = emptyList(),
    @EncodeDefault(EncodeDefault.Mode.ALWAYS) val opaque: List<String> = emptyList(),
    @EncodeDefault(EncodeDefault.Mode.ALWAYS) val lossy: List<String> = emptyList(),
    @EncodeDefault(EncodeDefault.Mode.ALWAYS) val unsupported: List<String> = emptyList(),
)

@Serializable
private data class ValidationCoverageWire(
    @EncodeDefault(EncodeDefault.Mode.ALWAYS) val exact: List<String> = emptyList(),
    @EncodeDefault(EncodeDefault.Mode.ALWAYS) val lossy: List<String> = emptyList(),
    @EncodeDefault(EncodeDefault.Mode.ALWAYS) val unsupported: List<String> = emptyList(),
)

internal object CoverageSerializer : KSerializer<Coverage> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("com.pons.ir.Coverage") {
            element<List<String>>("exact")
            element<List<String>>("opaque")
            element<List<String>>("lossy")
            element<List<String>>("unsupported")
        }

    override fun serialize(encoder: Encoder, value: Coverage) {
        encoder.encodeSerializableValue(
            CoverageWire.serializer(),
            CoverageWire(
                exact = value.exact.sorted(),
                opaque = value.opaque.sorted(),
                lossy = value.lossy.sorted(),
                unsupported = value.unsupported.sorted(),
            ),
        )
    }

    override fun deserialize(decoder: Decoder): Coverage {
        val wire = decoder.decodeSerializableValue(CoverageWire.serializer())
        return Coverage(
            exact = coverageLevel("exact", wire.exact),
            opaque = coverageLevel("opaque", wire.opaque),
            lossy = coverageLevel("lossy", wire.lossy),
            unsupported = coverageLevel("unsupported", wire.unsupported),
        )
    }
}

internal object ValidationCoverageSerializer : KSerializer<ValidationCoverage> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("com.pons.ir.ValidationCoverage") {
            element<List<String>>("exact")
            element<List<String>>("lossy")
            element<List<String>>("unsupported")
        }

    override fun serialize(encoder: Encoder, value: ValidationCoverage) {
        encoder.encodeSerializableValue(
            ValidationCoverageWire.serializer(),
            ValidationCoverageWire(
                exact = value.exact.sorted(),
                lossy = value.lossy.sorted(),
                unsupported = value.unsupported.sorted(),
            ),
        )
    }

    override fun deserialize(decoder: Decoder): ValidationCoverage {
        val wire = decoder.decodeSerializableValue(ValidationCoverageWire.serializer())
        return ValidationCoverage(
            exact = coverageLevel("exact", wire.exact),
            lossy = coverageLevel("lossy", wire.lossy),
            unsupported = coverageLevel("unsupported", wire.unsupported),
        )
    }
}

private fun coverageLevel(level: String, values: List<String>): Set<String> {
    val subjects = LinkedHashSet<String>()
    for (value in values) {
        if (!subjects.add(value)) {
            throw SerializationException(
                "coverage lists $value more than once in $level coverage",
            )
        }
    }
    return subjects
}
