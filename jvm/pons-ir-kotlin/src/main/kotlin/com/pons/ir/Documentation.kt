package com.pons.ir

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

@Serializable
data class Doc(
    val summary: String? = null,
    val description: String? = null,
    val deprecated: Boolean? = null,
    val tags: List<String> = emptyList(),
)

@Serializable
data class Diagnostic(
    val severity: DiagnosticSeverity,
    val code: String,
    val message: String,
    val origin: Origin? = null,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
enum class DiagnosticSeverity {
    @SerialName("error") Error,
    @SerialName("warning") Warning,
    @SerialName("info") Info,
}

@Serializable
data class Origin(
    val source: String? = null,
    val line: Long? = null,
    val column: Long? = null,
)
