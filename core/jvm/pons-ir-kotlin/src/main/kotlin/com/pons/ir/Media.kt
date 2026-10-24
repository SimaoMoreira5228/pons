package com.pons.ir

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class MediaDescriptor(
    @SerialName("type") val mediaType: String,
    val subtype: String,
    val parameters: Map<String, String> = emptyMap(),
    val codec: String? = null,
)


fun parseMediaDescriptor(value: String): MediaDescriptor? {
    val parts = value.split(';')
    val (type, subtype) = parts.first().trim().split('/', limit = 2)
        .takeIf { it.size == 2 } ?: return null
    if (type.isBlank() || subtype.isBlank()) return null
    val parameters = linkedMapOf<String, String>()
    for (parameter in parts.drop(1)) {
        val (name, raw) = parameter.trim().split('=', limit = 2)
            .takeIf { it.size == 2 } ?: return null
        if (name.isBlank() || raw.isBlank()) return null
        parameters[name.lowercase()] = raw.trim()
    }
    return MediaDescriptor(type.lowercase(), subtype.lowercase(), parameters)
}
