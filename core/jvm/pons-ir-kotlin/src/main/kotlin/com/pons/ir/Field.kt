@file:OptIn(kotlinx.serialization.InternalSerializationApi::class)

package com.pons.ir

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.descriptors.element
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement

@Serializable(with = FieldSerializer::class)
data class Field(
    val wire: String,
    val ident: Name,
    val ty: TypeRef,
    val presence: Presence,
    val default: FieldDefault = FieldDefault.Unspecified,
    val access: Access? = null,
    val doc: Doc? = null,
    val diagnostics: List<Diagnostic> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
)

sealed interface FieldDefault {
    data object Unspecified : FieldDefault

    data class Value(val value: JsonElement) : FieldDefault
}

@Serializable
enum class Presence {
    @SerialName("required") Required,
    @SerialName("optional") Optional,
}

@Serializable
enum class Access {
    @SerialName("read_write") ReadWrite,
    @SerialName("read_only") ReadOnly,
    @SerialName("write_only") WriteOnly,
}

@Serializable
enum class Direction {
    @SerialName("input") Input,
    @SerialName("output") Output,
    @SerialName("both") Both,
}

@Serializable
private data class FieldWire(
    val wire: String,
    val ident: Name,
    val ty: TypeRef,
    val presence: Presence,
    val default: JsonElement? = null,
    val access: Access? = null,
    val doc: Doc? = null,
    val diagnostics: List<Diagnostic> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
)

private val fieldKeys =
    setOf("wire", "ident", "ty", "presence", "default", "access", "doc", "diagnostics", "ext")

internal object FieldSerializer : KSerializer<Field> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("com.pons.ir.Field") {
        element<String>("wire")
        element<List<String>>("ident")
        element<TypeRef>("ty")
        element<Presence>("presence")
        element<JsonElement>("default")
        element<Access>("access")
        element<Doc>("doc")
        element<List<Diagnostic>>("diagnostics")
        element<Map<String, JsonElement>>("ext")
    }

    override fun serialize(encoder: Encoder, value: Field) {
        val jsonEncoder = encoder as? JsonEncoder
            ?: throw SerializationException("Field requires a JSON encoder")
        val json = jsonEncoder.json
        val objectValue = buildJsonObject {
            put("wire", JsonPrimitive(value.wire))
            put("ident", json.encodeToJsonElement(value.ident))
            put("ty", json.encodeToJsonElement(value.ty))
            put("presence", json.encodeToJsonElement(value.presence))
            val default = value.default
            if (default is FieldDefault.Value) {
                put("default", default.value)
            }
            value.access?.let { put("access", json.encodeToJsonElement(it)) }
            value.doc?.let { put("doc", json.encodeToJsonElement(it)) }
            if (value.diagnostics.isNotEmpty()) {
                put("diagnostics", json.encodeToJsonElement(value.diagnostics))
            }
            if (value.ext.isNotEmpty()) {
                put("ext", json.encodeToJsonElement(value.ext))
            }
        }
        jsonEncoder.encodeJsonElement(objectValue)
    }

    override fun deserialize(decoder: Decoder): Field {
        val jsonDecoder = decoder as? JsonDecoder
            ?: throw SerializationException("Field requires a JSON decoder")
        val element = jsonDecoder.decodeJsonElement() as? JsonObject
            ?: throw SerializationException("Field must be a JSON object")
        val unexpected = element.keys - fieldKeys
        if (unexpected.isNotEmpty()) {
            throw SerializationException("Field has unknown members: ${unexpected.sorted()}")
        }
        val json = jsonDecoder.json
        val wire = json.decodeFromJsonElement(FieldWire.serializer(), element)
        val default = if ("default" in element) {
            FieldDefault.Value(element.getValue("default"))
        } else {
            FieldDefault.Unspecified
        }
        return Field(
            wire = wire.wire,
            ident = wire.ident,
            ty = wire.ty,
            presence = wire.presence,
            default = default,
            access = wire.access,
            doc = wire.doc,
            diagnostics = wire.diagnostics,
            ext = wire.ext,
        )
    }
}
