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
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.descriptors.buildSerialDescriptor
import kotlinx.serialization.descriptors.element
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.jsonObject

@Serializable(with = TypeDefSerializer::class)
data class TypeDef(
    val name: Name,
    val direction: Direction,
    val definition: TypeKind,
    val origin: String? = null,
    val doc: Doc? = null,
    val diagnostics: List<Diagnostic> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
@JsonClassDiscriminator("kind")
sealed class TypeKind {
    @Serializable
    @SerialName("struct")
    data class StructType(
        val fields: List<Field> = emptyList(),
        val flattened: List<TypeRef> = emptyList(),
        val openness: ObjectOpenness,
    ) : TypeKind()

    @Serializable
    @SerialName("enum")
    data class EnumType(
        val repr: EnumRepr,
        val variants: List<Variant>,
    ) : TypeKind()

    @Serializable
    @SerialName("newtype")
    data class NewtypeType(val inner: TypeRef) : TypeKind()
}

@Serializable(with = ObjectOpennessSerializer::class)
sealed class ObjectOpenness {
    data object Reject : ObjectOpenness()

    data class PreserveTyped(val value: TypeRef) : ObjectOpenness()

    data object PreserveOpaque : ObjectOpenness()
}

@Serializable
@JsonClassDiscriminator("kind")
sealed class EnumRepr {
    @Serializable
    @SerialName("string")
    data object StringRepr : EnumRepr()

    @Serializable
    @SerialName("external")
    data object ExternalRepr : EnumRepr()

    @Serializable
    @SerialName("internal")
    data class InternalRepr(val tag: String) : EnumRepr()

    @Serializable
    @SerialName("adjacent")
    data class AdjacentRepr(val tag: String, val content: String) : EnumRepr()

    @Serializable
    @SerialName("untagged")
    data object UntaggedRepr : EnumRepr()
}

@Serializable
data class Variant(
    val wire: String,
    val ident: Name,
    val payload: VariantPayload,
    val doc: Doc? = null,
    val diagnostics: List<Diagnostic> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
@JsonClassDiscriminator("kind")
sealed class VariantPayload {
    @Serializable
    @SerialName("unit")
    data object UnitPayload : VariantPayload()

    @Serializable
    @SerialName("newtype")
    data class NewtypePayload(val ty: TypeRef) : VariantPayload()

    @Serializable
    @SerialName("tuple")
    data class TuplePayload(val items: List<TypeRef>) : VariantPayload()

    @Serializable
    @SerialName("struct")
    data class StructPayload(val fields: List<Field>) : VariantPayload()
}

@Serializable
private data class TypeDefCommon(
    val name: Name,
    val direction: Direction,
    val origin: String? = null,
    val doc: Doc? = null,
    val diagnostics: List<Diagnostic> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
)

private val typeDefCommonKeys =
    setOf("name", "direction", "origin", "doc", "diagnostics", "ext")

private val typeDefKindKeys = mapOf(
    "struct" to setOf("fields", "flattened", "openness"),
    "enum" to setOf("repr", "variants"),
    "newtype" to setOf("inner"),
)

internal object TypeDefSerializer : KSerializer<TypeDef> {
    override val descriptor: SerialDescriptor =
        buildClassSerialDescriptor("com.pons.ir.TypeDef") {
            element<List<String>>("name")
            element<Direction>("direction")
            element<String>("kind")
        }

    override fun serialize(encoder: Encoder, value: TypeDef) {
        val jsonEncoder = encoder as? JsonEncoder
            ?: throw SerializationException("TypeDef requires a JSON encoder")
        val json = jsonEncoder.json
        val common = json.encodeToJsonElement(
            TypeDefCommon.serializer(),
            TypeDefCommon(
                name = value.name,
                direction = value.direction,
                origin = value.origin,
                doc = value.doc,
                diagnostics = value.diagnostics,
                ext = value.ext,
            ),
        ).jsonObject
        val definition = json.encodeToJsonElement(TypeKind.serializer(), value.definition).jsonObject
        jsonEncoder.encodeJsonElement(JsonObject(common + definition))
    }

    override fun deserialize(decoder: Decoder): TypeDef {
        val jsonDecoder = decoder as? JsonDecoder
            ?: throw SerializationException("TypeDef requires a JSON decoder")
        val element = jsonDecoder.decodeJsonElement() as? JsonObject
            ?: throw SerializationException("TypeDef must be a JSON object")
        val discriminator = element["kind"] as? JsonPrimitive
        val kind = discriminator?.contentOrNull
            ?: throw SerializationException("TypeDef requires a string kind")
        val kindKeys = typeDefKindKeys[kind].orEmpty()
        val unexpected = element.keys - (typeDefCommonKeys + "kind" + kindKeys)
        if (unexpected.isNotEmpty()) {
            throw SerializationException("TypeDef has unknown members: ${unexpected.sorted()}")
        }
        val json = jsonDecoder.json
        val common = json.decodeFromJsonElement(
            TypeDefCommon.serializer(),
            JsonObject(element.filterKeys { it in typeDefCommonKeys }),
        )
        val definition = json.decodeFromJsonElement(
            TypeKind.serializer(),
            JsonObject(element.filterKeys { it == "kind" || it in kindKeys }),
        )
        return TypeDef(
            name = common.name,
            direction = common.direction,
            definition = definition,
            origin = common.origin,
            doc = common.doc,
            diagnostics = common.diagnostics,
            ext = common.ext,
        )
    }
}

internal object ObjectOpennessSerializer : KSerializer<ObjectOpenness> {
    override val descriptor: SerialDescriptor =
        buildSerialDescriptor("com.pons.ir.ObjectOpenness", SerialKind.CONTEXTUAL)

    override fun serialize(encoder: Encoder, value: ObjectOpenness) {
        val jsonEncoder = encoder as? JsonEncoder
            ?: throw SerializationException("ObjectOpenness requires a JSON encoder")
        val element = when (value) {
            ObjectOpenness.Reject -> JsonPrimitive("reject")
            ObjectOpenness.PreserveOpaque -> JsonPrimitive("preserve_opaque")
            is ObjectOpenness.PreserveTyped -> JsonObject(
                mapOf(
                    "preserve_typed" to JsonObject(
                        mapOf("value" to jsonEncoder.json.encodeToJsonElement(value.value)),
                    ),
                ),
            )
        }
        jsonEncoder.encodeJsonElement(element)
    }

    override fun deserialize(decoder: Decoder): ObjectOpenness {
        val jsonDecoder = decoder as? JsonDecoder
            ?: throw SerializationException("ObjectOpenness requires a JSON decoder")
        return when (val element = jsonDecoder.decodeJsonElement()) {
            is JsonPrimitive -> when (element.contentOrNull) {
                "reject" -> ObjectOpenness.Reject
                "preserve_opaque" -> ObjectOpenness.PreserveOpaque
                else -> throw SerializationException("Unknown object openness ${element.content}")
            }
            is JsonObject -> {
                if (element.keys != setOf("preserve_typed")) {
                    throw SerializationException("Object openness must be a string or preserve_typed")
                }
                val wrapped = element.getValue("preserve_typed").jsonObject
                if (wrapped.keys != setOf("value")) {
                    throw SerializationException("preserve_typed requires exactly one value member")
                }
                ObjectOpenness.PreserveTyped(
                    jsonDecoder.json.decodeFromJsonElement(wrapped.getValue("value")),
                )
            }
            else -> throw SerializationException("Object openness must be a string or object")
        }
    }
}
