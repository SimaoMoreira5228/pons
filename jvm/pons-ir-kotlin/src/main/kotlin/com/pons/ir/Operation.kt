@file:OptIn(kotlinx.serialization.InternalSerializationApi::class, kotlinx.serialization.ExperimentalSerializationApi::class)

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
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.decodeFromJsonElement

@Serializable
data class Operation(
    val id: List<Name>,
    val method: Method,
    val path: List<PathSegment>,
    val query: TypeRef? = null,
    val headers: List<HeaderParameter> = emptyList(),
    val body: Body? = null,
    val responses: List<Response> = emptyList(),
    val errors: List<ErrorResponse> = emptyList(),
    @SerialName("errors_inherit") val errorsInherit: Boolean = true,
    val stream: StreamKind? = null,
    val auth: AuthRequirement,
    val provides: CredentialProvision? = null,
    val semantics: OperationSemantics,
    val doc: Doc? = null,
    val origin: Origin? = null,
    val ext: Map<String, JsonElement> = emptyMap(),
    val diagnostics: List<Diagnostic> = emptyList(),
)

@Serializable
enum class Method {
    @SerialName("GET") Get,
    @SerialName("POST") Post,
    @SerialName("PUT") Put,
    @SerialName("PATCH") Patch,
    @SerialName("DELETE") Delete,
    @SerialName("HEAD") Head,
    @SerialName("OPTIONS") Options,
}

@Serializable(with = PathSegmentSerializer::class)
sealed class PathSegment {
    data class Literal(val lit: String) : PathSegment()

    data class Param(val param: String, val ty: TypeRef) : PathSegment()
}

@Serializable
data class HeaderParameter(
    val wire: String,
    val ident: Name,
    val ty: TypeRef,
    val presence: Presence,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
data class Body(
    val ty: TypeRef,
    val content: ContentKind,
    val required: Boolean,
    @SerialName("media_type") val mediaType: String? = null,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
enum class ContentKind {
    @SerialName("json") Json,
    @SerialName("form") Form,
    @SerialName("multipart") Multipart,
    @SerialName("text") Text,
    @SerialName("bytes") Bytes,
    @SerialName("stream") Stream,
    @SerialName("problem_json") ProblemJson,
    @SerialName("empty") Empty,
}

@Serializable
data class Response(
    val status: ResponseStatus,
    val content: ContentKind,
    @SerialName("media_type") val mediaType: String? = null,
    val body: TypeRef? = null,
    val headers: List<ResponseHeader> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
data class ResponseHeader(
    val wire: String,
    val ty: TypeRef,
    val presence: Presence,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
data class ErrorResponse(
    val status: ErrorStatus,
    val name: Name,
    val content: ErrorContent,
    @SerialName("media_type") val mediaType: String? = null,
    val body: TypeRef? = null,
    val headers: List<ResponseHeader> = emptyList(),
    val retryable: Boolean? = null,
    val doc: Doc? = null,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
enum class ErrorContent {
    @SerialName("json") Json,
    @SerialName("problem_json") ProblemJson,
    @SerialName("text") Text,
    @SerialName("empty") Empty,
}

@Serializable
data class OperationSemantics(
    val safe: Boolean,
    val idempotent: Boolean,
    val pagination: Pagination? = null,
)

@Serializable
data class Pagination(
    val request: PaginationRequest,
    val items: String,
    val next: String? = null,
    @SerialName("page_size") val pageSize: PageSize? = null,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
@JsonClassDiscriminator("kind")
sealed class PaginationRequest {
    abstract val field: String

    @Serializable
    @SerialName("cursor")
    data class Cursor(override val field: String) : PaginationRequest()

    @Serializable
    @SerialName("offset")
    data class Offset(override val field: String) : PaginationRequest()
}

@Serializable
data class PageSize(
    val field: String,
    val default: UInt? = null,
    val maximum: UInt? = null,
)

internal object PathSegmentSerializer : KSerializer<PathSegment> {
    override val descriptor: SerialDescriptor =
        buildSerialDescriptor("com.pons.ir.PathSegment", SerialKind.CONTEXTUAL)

    override fun serialize(encoder: Encoder, value: PathSegment) {
        when (value) {
            is PathSegment.Literal -> encoder.encodeSerializableValue(
                PathSegmentLiteralWire.serializer(),
                PathSegmentLiteralWire(value.lit),
            )
            is PathSegment.Param -> encoder.encodeSerializableValue(
                PathSegmentParamWire.serializer(),
                PathSegmentParamWire(value.param, value.ty),
            )
        }
    }

    override fun deserialize(decoder: Decoder): PathSegment {
        val jsonDecoder = decoder as? JsonDecoder
            ?: throw SerializationException("PathSegment requires a JSON decoder")
        val element = jsonDecoder.decodeJsonElement() as? JsonObject
            ?: throw SerializationException("PathSegment must be a JSON object")
        val json = jsonDecoder.json
        return if ("lit" in element) {
            val wire = json.decodeFromJsonElement(PathSegmentLiteralWire.serializer(), element)
            PathSegment.Literal(wire.lit)
        } else {
            val wire = json.decodeFromJsonElement(PathSegmentParamWire.serializer(), element)
            PathSegment.Param(wire.param, wire.ty)
        }
    }
}

@Serializable
private data class PathSegmentLiteralWire(val lit: String)

@Serializable
private data class PathSegmentParamWire(val param: String, val ty: TypeRef)
