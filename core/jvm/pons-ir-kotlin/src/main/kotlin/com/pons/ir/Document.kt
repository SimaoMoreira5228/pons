package com.pons.ir

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

typealias Name = List<String>

@Serializable
data class Document(
    @SerialName("ir_version") val irVersion: String,
    val info: Info,
    val types: Map<String, TypeDef> = emptyMap(),
    @SerialName("auth_schemes") val authSchemes: Map<String, AuthScheme> = emptyMap(),
    @SerialName("common_errors") val commonErrors: List<ErrorResponse> = emptyList(),
    val operations: List<Operation> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
    val diagnostics: List<Diagnostic> = emptyList(),
)

@Serializable
data class Info(
    val title: String,
    val version: String,
    val description: String? = null,
    val servers: List<String> = emptyList(),
    val ext: Map<String, JsonElement> = emptyMap(),
)
