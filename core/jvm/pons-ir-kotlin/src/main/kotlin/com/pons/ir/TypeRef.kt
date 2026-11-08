@file:OptIn(
    kotlinx.serialization.InternalSerializationApi::class,
    kotlinx.serialization.ExperimentalSerializationApi::class,
)

package com.pons.ir

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.SerialKind
import kotlinx.serialization.descriptors.buildSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.decodeFromJsonElement

@Serializable(with = TypeRefSerializer::class)
sealed class TypeRef {
    data class Named(val ref: NamedRef) : TypeRef()

    data class Inline(val type: InlineType) : TypeRef()
}

@Serializable
data class NamedRef(
    @SerialName("ref") val id: String,
    val variant: String? = null,
)

@Serializable
@JsonClassDiscriminator("kind")
sealed class InlineType {
    @Serializable
    @SerialName("bool")
    data object BoolType : InlineType()

    @Serializable
    @SerialName("int")
    data class IntType(
        val bits: Int,
        val signed: Boolean,
        val wire: IntegerWire,
        val minimum: JsonElement? = null,
        val maximum: JsonElement? = null,
        @SerialName("multiple_of") val multipleOf: JsonElement? = null,
    ) : InlineType()

    @Serializable
    @SerialName("float")
    data class FloatType(val bits: Int) : InlineType()

    @Serializable
    @SerialName("decimal")
    data class DecimalType(val wire: DecimalWire) : InlineType()

    @Serializable
    @SerialName("string")
    data class StringType(
        val format: FieldFormat? = null,
        @SerialName("min_length") val minLength: Long? = null,
        @SerialName("max_length") val maxLength: Long? = null,
        val pattern: String? = null,
    ) : InlineType()

    @Serializable
    @SerialName("bytes")
    data class BytesType(val encoding: ByteEncoding) : InlineType()

    @Serializable
    @SerialName("null")
    data object NullType : InlineType()

    @Serializable
    @SerialName("any")
    data object AnyType : InlineType()

    @Serializable
    @SerialName("opaque")
    data class OpaqueType(
        val codec: String,
        val wire: OpaqueWire,
        val ext: Map<String, JsonElement> = emptyMap(),
    ) : InlineType()

    @Serializable
    @SerialName("nullable")
    data class NullableType(val inner: TypeRef) : InlineType()

    @Serializable
    @SerialName("array")
    data class ArrayType(
        val item: TypeRef,
        val unique: Boolean? = null,
    ) : InlineType()

    @Serializable
    @SerialName("map")
    data class MapType(
        val key: TypeRef,
        val value: TypeRef,
    ) : InlineType()

    @Serializable
    @SerialName("tuple")
    data class TupleType(val items: List<TypeRef>) : InlineType()

    @Serializable
    @SerialName("struct")
    data class StructType(
        val fields: List<Field>,
        val flattened: List<TypeRef> = emptyList(),
        val openness: ObjectOpenness,
    ) : InlineType()
}

@Serializable
enum class IntegerWire {
    @SerialName("number") Number,
    @SerialName("string") String,
}

@Serializable
enum class DecimalWire {
    @SerialName("number") Number,
    @SerialName("string") String,
}

@Serializable
enum class OpaqueWire {
    @SerialName("json") Json,
    @SerialName("text") Text,
    @SerialName("bytes") Bytes,
    @SerialName("unknown") Unknown,
}

@Serializable
enum class ByteEncoding {
    @SerialName("base64") Base64,
    @SerialName("base64url") Base64url,
    @SerialName("hex") Hex,
    @SerialName("array") Array,
}

@Serializable
enum class FieldFormat {
    @SerialName("uuid") Uuid,
    @SerialName("date-time") DateTime,
    @SerialName("date") Date,
    @SerialName("time") Time,
    @SerialName("duration") Duration,
    @SerialName("uri") Uri,
    @SerialName("email") Email,
    @SerialName("ipv4") Ipv4,
    @SerialName("ipv6") Ipv6,
}

internal object TypeRefSerializer : KSerializer<TypeRef> {
    override val descriptor: SerialDescriptor =
        buildSerialDescriptor("com.pons.ir.TypeRef", SerialKind.CONTEXTUAL)

    override fun serialize(encoder: Encoder, value: TypeRef) {
        when (value) {
            is TypeRef.Named -> encoder.encodeSerializableValue(NamedRef.serializer(), value.ref)
            is TypeRef.Inline -> encoder.encodeSerializableValue(InlineType.serializer(), value.type)
        }
    }

    override fun deserialize(decoder: Decoder): TypeRef {
        val jsonDecoder = decoder as? JsonDecoder
            ?: throw SerializationException("TypeRef requires a JSON decoder")
        val element = jsonDecoder.decodeJsonElement()
        val objectElement = element as? JsonObject
            ?: throw SerializationException("TypeRef must be a JSON object")
        val json = jsonDecoder.json
        return if ("ref" in objectElement) {
            TypeRef.Named(json.decodeFromJsonElement(NamedRef.serializer(), objectElement))
        } else {
            TypeRef.Inline(json.decodeFromJsonElement(InlineType.serializer(), objectElement))
        }
    }
}
