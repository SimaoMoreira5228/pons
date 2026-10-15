package com.pons.spring

import com.fasterxml.jackson.databind.JsonNode
import com.fasterxml.jackson.databind.ObjectMapper
import com.pons.ir.ContentKind
import com.pons.ir.Direction
import com.pons.ir.Document
import com.pons.ir.ErrorStatus
import com.pons.ir.InlineType
import com.pons.ir.Method
import com.pons.ir.Operation
import com.pons.ir.PathSegment
import com.pons.ir.Presence
import com.pons.ir.TypeKind
import com.pons.ir.TypeRef
import com.pons.ir.Violation
import com.pons.ir.validateValue
import jakarta.servlet.http.HttpServletRequest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.math.BigDecimal

internal class PonsRequestValidator(
    private val mapper: ObjectMapper,
) {
    fun operation(document: Document, method: String, path: String): Operation? {
        val sent = Method.entries.firstOrNull { it.name.equals(method, ignoreCase = true) }
            ?: return null
        val parts = pathSegments(path)
        return document.operations
            .filter { it.method == sent && pathMatches(it.path, parts) }
            .maxByOrNull { specificity(it.path) }
    }

    fun rejects(operation: Operation): Boolean =
        operation.errors.any {
            it.status == ErrorStatus.Exact(REJECTED_STATUS) && it.name == REJECTED_NAME
        }

    fun contentViolation(
        operation: Operation,
        request: HttpServletRequest,
        body: ByteArray?,
    ): Violation? {
        val declared = operation.body ?: return null
        if (body == null || body.isEmpty()) return null
        val expected = when (declared.content) {
            ContentKind.Json -> "application/json"
            ContentKind.Form -> "application/x-www-form-urlencoded"
            else -> return null
        }
        val found = request.contentType?.substringBefore(';')?.trim()?.lowercase()
        val matches = when (declared.content) {
            ContentKind.Json -> found == expected || found?.endsWith("+json") == true
            else -> found == expected
        }
        if (matches) return null
        return Violation("", "expected $expected, found ${found ?: "no content type"}")
    }

    fun readsBody(operation: Operation, request: HttpServletRequest): Boolean {
        val declared = operation.body ?: return false
        val empty = request.contentLengthLong == 0L
        return when (declared.content) {
            ContentKind.Json -> !empty
            ContentKind.Form -> !empty && request.method !in setOf("GET", "HEAD")
            else -> false
        }
    }

    fun validate(
        document: Document,
        operation: Operation,
        request: HttpServletRequest,
        path: String,
        body: ByteArray?,
    ): List<Violation> {
        val violations = mutableListOf<Violation>()
        violations += pathViolations(document, operation, path)
        violations += queryViolations(document, operation, request)
        violations += headerViolations(document, operation, request)
        violations += bodyViolations(document, operation, request, body)
        return violations
    }

    private fun pathViolations(
        document: Document,
        operation: Operation,
        path: String,
    ): List<Violation> {
        val parts = pathSegments(path)
        val violations = mutableListOf<Violation>()
        operation.path.forEachIndexed { index, segment ->
            if (segment !is PathSegment.Param) return@forEachIndexed
            val base = child("", segment.param)
            val text = parts.getOrNull(index) ?: return@forEachIndexed
            val value = tokenValue(document, segment.ty, text)
            violations += validateValue(document, segment.ty, Direction.Input, value)
                .map { placed(base, it) }
        }
        return violations
    }

    private fun queryViolations(
        document: Document,
        operation: Operation,
        request: HttpServletRequest,
    ): List<Violation> {
        val query = operation.query ?: return emptyList()
        val (value, violations) = objectValue(
            document,
            query,
            decodePairs(request.queryString.orEmpty()),
        )
        return violations + validateValue(document, query, Direction.Input, value)
    }

    private fun headerViolations(
        document: Document,
        operation: Operation,
        request: HttpServletRequest,
    ): List<Violation> {
        val violations = mutableListOf<Violation>()
        for (header in operation.headers) {
            val path = child("", header.wire)
            val values = request.getHeaders(header.wire).toList()
            if (values.isEmpty()) {
                if (header.presence == Presence.Required) {
                    violations += Violation(path, "missing required field ${header.wire}")
                }
                continue
            }
            if (values.size > 1) {
                violations += Violation(
                    path,
                    "expected one value for a field that is not an array, found ${values.size} values",
                )
                continue
            }
            val value = tokenValue(document, header.ty, values.single())
            violations += validateValue(document, header.ty, Direction.Input, value)
                .map { placed(path, it) }
        }
        return violations
    }

    private fun bodyViolations(
        document: Document,
        operation: Operation,
        request: HttpServletRequest,
        body: ByteArray?,
    ): List<Violation> {
        val declared = operation.body ?: return emptyList()
        return when (declared.content) {
            ContentKind.Json -> jsonViolations(document, declared.ty, declared.required, body)
            ContentKind.Form -> formViolations(
                document,
                declared.ty,
                declared.required,
                request,
                body,
            )
            else -> emptyList()
        }
    }

    private fun jsonViolations(
        document: Document,
        ty: TypeRef,
        required: Boolean,
        body: ByteArray?,
    ): List<Violation> {
        if (body == null || body.isEmpty()) {
            return if (required) {
                listOf(Violation("", "expected a JSON document, found no body"))
            } else {
                emptyList()
            }
        }
        val parsed = try {
            mapper.readTree(body)
        } catch (error: Exception) {
            return listOf(
                Violation("", "expected a JSON document, found ${error.message ?: error.javaClass.name}"),
            )
        }
        return validateValue(document, ty, Direction.Input, parsed.toJsonElement())
    }

    private fun formViolations(
        document: Document,
        ty: TypeRef,
        required: Boolean,
        request: HttpServletRequest,
        body: ByteArray?,
    ): List<Violation> {
        if (request.method !in setOf("GET", "HEAD") && (body == null || body.isEmpty()) && required) {
            return listOf(Violation("", "expected a form document, found no body"))
        }
        val text = when (request.method) {
            "GET", "HEAD" -> request.queryString.orEmpty()
            else -> body?.toString(charset(request)).orEmpty()
        }
        val (value, violations) = objectValue(document, ty, decodePairs(text))
        return violations + validateValue(document, ty, Direction.Input, value)
    }

    private fun objectValue(
        document: Document,
        ty: TypeRef,
        pairs: List<Pair<String, String>>,
    ): Pair<JsonElement, List<Violation>> {
        val declared = mutableListOf<Pair<String, TypeRef>>()
        members(document, ty, declared, mutableListOf())
        val entries = LinkedHashMap<String, JsonElement>()
        val violations = mutableListOf<Violation>()
        for ((wire, texts) in grouped(pairs)) {
            val field = declared.firstOrNull { it.first == wire }?.second
            val item = field?.let { arrayItem(document, it) }
            val value = when {
                field != null && item != null -> arrayValue(document, item, texts)
                field != null && texts.size > 1 -> {
                    violations += Violation(
                        child("", wire),
                        "expected one value for a field that is not an array, found ${texts.size} values",
                    )
                    tokenValue(document, field, texts.last())
                }
                field != null -> tokenValue(document, field, texts.first())
                else -> JsonPrimitive(texts.last())
            }
            entries[wire] = value
        }
        return JsonObject(entries) to violations
    }

    private fun arrayValue(
        document: Document,
        item: TypeRef,
        texts: List<String>,
    ): JsonElement = JsonArray(
        texts.flatMap { text ->
            when (val spelled = spellsArray(text)) {
                null -> listOf(itemValue(document, item, text))
                else -> spelled
            }
        },
    )

    private fun itemValue(document: Document, item: TypeRef, text: String): JsonElement {
        if (!carriesJson(document, item)) return tokenValue(document, item, text)
        return runCatching { Json.parseToJsonElement(text) }.getOrElse { JsonPrimitive(text) }
    }

    private fun spellsArray(text: String): List<JsonElement>? {
        val parsed = runCatching { Json.parseToJsonElement(text) }.getOrNull()
        return (parsed as? JsonArray)?.toList()
    }

    private fun members(
        document: Document,
        ty: TypeRef,
        found: MutableList<Pair<String, TypeRef>>,
        seen: MutableList<String>,
    ) {
        val named = (ty as? TypeRef.Named)?.ref ?: return
        if (seen.contains(named.id)) return
        val definition = document.types[named.id] ?: return
        seen.add(named.id)
        when (val kind = definition.definition) {
            is TypeKind.NewtypeType -> members(document, kind.inner, found, seen)
            is TypeKind.StructType -> {
                found += kind.fields.map { it.wire to it.ty }
                kind.flattened.forEach { members(document, it, found, seen) }
            }
            is TypeKind.EnumType -> {}
        }
        seen.remove(named.id)
    }

    private fun tokenValue(document: Document, ty: TypeRef, text: String): JsonElement {
        val item = arrayItem(document, ty)
        if (item != null) {
            val spelled = spellsArray(text)
            return JsonArray(spelled ?: listOf(tokenValue(document, item, text)))
        }
        return when (ty) {
            is TypeRef.Named -> {
                val definition = document.types[ty.ref.id]?.definition
                if (definition is TypeKind.NewtypeType) {
                    tokenValue(document, definition.inner, text)
                } else {
                    JsonPrimitive(text)
                }
            }
            is TypeRef.Inline -> when (val inline = ty.type) {
                is InlineType.NullableType -> tokenValue(document, inline.inner, text)
                InlineType.BoolType -> when (text) {
                    "true" -> JsonPrimitive(true)
                    "false" -> JsonPrimitive(false)
                    else -> JsonPrimitive(text)
                }
                is InlineType.IntType -> if (inline.wire == com.pons.ir.IntegerWire.Number) {
                    numberOrText(text)
                } else {
                    JsonPrimitive(text)
                }
                is InlineType.FloatType -> numberOrText(text)
                is InlineType.DecimalType -> if (inline.wire == com.pons.ir.DecimalWire.Number) {
                    numberOrText(text)
                } else {
                    JsonPrimitive(text)
                }
                else -> JsonPrimitive(text)
            }
        }
    }

    private fun numberOrText(text: String): JsonElement =
        text.toBigDecimalOrNull()?.let { JsonPrimitive(it) } ?: JsonPrimitive(text)

    private fun arrayItem(document: Document, ty: TypeRef): TypeRef? = when (ty) {
        is TypeRef.Named -> {
            val definition = document.types[ty.ref.id]?.definition
            if (definition is TypeKind.NewtypeType) {
                arrayItem(document, definition.inner)
            } else {
                null
            }
        }
        is TypeRef.Inline -> when (val inline = ty.type) {
            is InlineType.NullableType -> arrayItem(document, inline.inner)
            is InlineType.ArrayType -> inline.item
            else -> null
        }
    }

    private fun carriesJson(document: Document, ty: TypeRef): Boolean = when (ty) {
        is TypeRef.Inline -> when (val inline = ty.type) {
            InlineType.AnyType,
            is InlineType.MapType,
            is InlineType.TupleType,
            -> true
            is InlineType.NullableType -> carriesJson(document, inline.inner)
            else -> false
        }
        is TypeRef.Named -> when (val definition = document.types[ty.ref.id]?.definition) {
            is TypeKind.StructType -> true
            is TypeKind.NewtypeType -> carriesJson(document, definition.inner)
            else -> false
        }
    }

    private fun grouped(pairs: List<Pair<String, String>>): List<Pair<String, List<String>>> {
        val groups = mutableListOf<Pair<String, MutableList<String>>>()
        for ((wire, text) in pairs) {
            val found = groups.firstOrNull { it.first == wire }
            if (found != null) {
                found.second.add(text)
            } else {
                groups += wire to mutableListOf(text)
            }
        }
        return groups.map { it.first to it.second.toList() }
    }

    private fun charset(request: HttpServletRequest): java.nio.charset.Charset =
        request.characterEncoding?.let { runCatching { java.nio.charset.Charset.forName(it) }.getOrNull() }
            ?: Charsets.UTF_8

    private fun specificity(segments: List<PathSegment>): Specificity =
        Specificity(
            segments.size,
            segments.map { if (it is PathSegment.Literal) 1 else 0 },
        )
}

private data class Specificity(
    val length: Int,
    val literals: List<Int>,
) : Comparable<Specificity> {
    override fun compareTo(other: Specificity): Int {
        val length = length.compareTo(other.length)
        if (length != 0) return length
        for (index in 0 until minOf(literals.size, other.literals.size)) {
            val comparison = literals[index].compareTo(other.literals[index])
            if (comparison != 0) return comparison
        }
        return literals.size.compareTo(other.literals.size)
    }
}

private fun pathMatches(segments: List<PathSegment>, parts: List<String>): Boolean {
    if (segments.size != parts.size) return false
    return segments.withIndex().all { (index, segment) ->
        when (segment) {
            is PathSegment.Literal -> segment.lit == parts[index]
            is PathSegment.Param -> index < segments.lastIndex || parts[index].isNotEmpty()
        }
    }
}

internal fun pathSegments(path: String): List<String> {
    val stripped = if (path.startsWith('/')) path.substring(1) else path
    return if (stripped.isEmpty()) emptyList() else stripped.split('/')
}

private fun child(path: String, token: String): String =
    "$path/${token.replace("~", "~0").replace("/", "~1")}"

private fun placed(base: String, violation: Violation): Violation =
    if (violation.path.isEmpty()) Violation(base, violation.reason) else Violation(base + violation.path, violation.reason)

internal fun decodePairs(text: String): List<Pair<String, String>> =
    text.split('&')
        .filter { it.isNotEmpty() }
        .map { pair ->
            val index = pair.indexOf('=')
            if (index < 0) {
                decode(pair) to ""
            } else {
                decode(pair.substring(0, index)) to decode(pair.substring(index + 1))
            }
        }

private fun decode(text: String): String {
    if ('%' !in text && '+' !in text) return text
    val bytes = text.toByteArray(Charsets.UTF_8)
    val decoded = java.io.ByteArrayOutputStream(bytes.size)
    var index = 0
    while (index < bytes.size) {
        val byte = bytes[index].toInt() and 0xff
        when {
            byte == '+'.code -> {
                decoded.write(' '.code)
                index += 1
            }
            byte == '%'.code && index + 2 < bytes.size -> {
                val high = nibble(bytes[index + 1])
                val low = nibble(bytes[index + 2])
                if (high >= 0 && low >= 0) {
                    decoded.write(high shl 4 or low)
                    index += 3
                } else {
                    decoded.write(byte)
                    index += 1
                }
            }
            else -> {
                decoded.write(byte)
                index += 1
            }
        }
    }
    return decoded.toString(Charsets.UTF_8)
}

private fun nibble(byte: Byte): Int = when (val value = byte.toInt() and 0xff) {
    in '0'.code..'9'.code -> value - '0'.code
    in 'a'.code..'f'.code -> value - 'a'.code + 10
    in 'A'.code..'F'.code -> value - 'A'.code + 10
    else -> -1
}

private fun JsonNode.toJsonElement(): JsonElement = when {
    isNull -> JsonNull
    isBoolean -> JsonPrimitive(booleanValue())
    isNumber -> if (isIntegralNumber) {
        JsonPrimitive(bigIntegerValue())
    } else {
        JsonPrimitive(BigDecimal(asText()))
    }
    isTextual -> JsonPrimitive(textValue())
    isArray -> JsonArray(map { it.toJsonElement() })
    isObject -> JsonObject(properties().asSequence().associate { it.key to it.value.toJsonElement() })
    else -> JsonPrimitive(asText())
}
