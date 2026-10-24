@file:OptIn(kotlinx.serialization.InternalSerializationApi::class)

package com.pons.ir

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerializationException
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.SerialKind
import kotlinx.serialization.descriptors.buildSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull

@Serializable(with = ResponseStatusSerializer::class)
sealed class ResponseStatus {
    data class Exact(val code: Int) : ResponseStatus()

    data class ClassStatus(val code: Int) : ResponseStatus()

    data object Default : ResponseStatus()

    data object Unknown : ResponseStatus()
}

@Serializable(with = ErrorStatusSerializer::class)
sealed class ErrorStatus {
    data class Exact(val code: Int) : ErrorStatus()

    data class ClassStatus(val code: Int) : ErrorStatus()

    data object Default : ErrorStatus()
}

private val statusClasses = (1..5).associateBy { "${it}xx" }

internal object ResponseStatusSerializer : KSerializer<ResponseStatus> {
    override val descriptor: SerialDescriptor =
        buildSerialDescriptor("com.pons.ir.ResponseStatus", SerialKind.CONTEXTUAL)

    override fun serialize(encoder: Encoder, value: ResponseStatus) {
        val jsonEncoder = encoder as? JsonEncoder
            ?: throw SerializationException("ResponseStatus requires a JSON encoder")
        val element = when (value) {
            is ResponseStatus.Exact -> JsonPrimitive(value.code)
            is ResponseStatus.ClassStatus -> JsonPrimitive("${value.code}xx")
            ResponseStatus.Default -> JsonPrimitive("default")
            ResponseStatus.Unknown -> JsonPrimitive("unknown")
        }
        jsonEncoder.encodeJsonElement(element)
    }

    override fun deserialize(decoder: Decoder): ResponseStatus {
        val jsonDecoder = decoder as? JsonDecoder
            ?: throw SerializationException("ResponseStatus requires a JSON decoder")
        val element = jsonDecoder.decodeJsonElement() as? JsonPrimitive
            ?: throw SerializationException("ResponseStatus must be an integer or string")
        element.intOrNull?.let { return ResponseStatus.Exact(it) }
        return when (val name = element.contentOrNull) {
            "default" -> ResponseStatus.Default
            "unknown" -> ResponseStatus.Unknown
            null -> throw SerializationException("ResponseStatus must be an integer or string")
            else -> statusClasses[name]?.let { ResponseStatus.ClassStatus(it) }
                ?: throw SerializationException("Unknown response status $name")
        }
    }
}

internal object ErrorStatusSerializer : KSerializer<ErrorStatus> {
    override val descriptor: SerialDescriptor =
        buildSerialDescriptor("com.pons.ir.ErrorStatus", SerialKind.CONTEXTUAL)

    override fun serialize(encoder: Encoder, value: ErrorStatus) {
        val jsonEncoder = encoder as? JsonEncoder
            ?: throw SerializationException("ErrorStatus requires a JSON encoder")
        val element = when (value) {
            is ErrorStatus.Exact -> JsonPrimitive(value.code)
            is ErrorStatus.ClassStatus -> JsonPrimitive("${value.code}xx")
            ErrorStatus.Default -> JsonPrimitive("default")
        }
        jsonEncoder.encodeJsonElement(element)
    }

    override fun deserialize(decoder: Decoder): ErrorStatus {
        val jsonDecoder = decoder as? JsonDecoder
            ?: throw SerializationException("ErrorStatus requires a JSON decoder")
        val element = jsonDecoder.decodeJsonElement() as? JsonPrimitive
            ?: throw SerializationException("ErrorStatus must be an integer or string")
        element.intOrNull?.let { return ErrorStatus.Exact(it) }
        return when (val name = element.contentOrNull) {
            "default" -> ErrorStatus.Default
            null -> throw SerializationException("ErrorStatus must be an integer or string")
            else -> statusClasses[name]?.let { ErrorStatus.ClassStatus(it) }
                ?: throw SerializationException("Unknown error status $name")
        }
    }
}
