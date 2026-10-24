@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.pons.ir

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonClassDiscriminator

@Serializable
@JsonClassDiscriminator("kind")
sealed class StreamKind {
    @Serializable
    @SerialName("sse")
    data class SseStream(val events: List<StreamMessage>) : StreamKind()

    @Serializable
    @SerialName("ndjson")
    data class NdjsonStream(val item: TypeRef) : StreamKind()

    @Serializable
    @SerialName("bytes")
    data class BytesStream(@SerialName("content_type") val contentType: String) : StreamKind()

    @Serializable
    @SerialName("websocket")
    data class WebsocketStream(
        val client: TypeRef,
        val server: TypeRef,
    ) : StreamKind()
}

@Serializable
data class StreamMessage(
    val event: String,
    val data: TypeRef,
)
